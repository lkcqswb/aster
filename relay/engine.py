"""Bounded agent loop, append-only traces, and reproducible checkpoints."""
import copy
import fcntl
import json
import queue
import threading
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path

from .providers import DemoProvider, MiniMaxProvider, ProviderError
from .tasks import TASKS, validate_task
from .tools import TOOL_SCHEMAS, execute, snapshot, verify, write_file

TERMINAL = {"passed", "failed", "unverified", "error", "cancelled", "limit", "interrupted"}
DEFAULT_LIMITS = {"max_steps": 12, "max_tools": 24, "max_seconds": 180,
                  "max_output_tokens": 2048, "total_output_tokens": 12000}
RANGES = {"max_steps": (1, 30), "max_tools": (1, 60), "max_seconds": (1, 600),
          "max_output_tokens": (64, 4096), "total_output_tokens": (64, 32768)}


def now():
    return datetime.now(timezone.utc).isoformat()


def dump(path: Path, data):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(data, ensure_ascii=False, indent=2, allow_nan=False), encoding="utf-8")
    temporary.replace(path)


class StopRun(Exception):
    def __init__(self, status, message):
        self.status, self.message = status, message


class Engine:
    def __init__(self, root, provider_factory=None):
        self.root = Path(root).resolve()
        self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        self._lock_handle = (self.root / ".engine.lock").open("a")
        try:
            fcntl.flock(self._lock_handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            self._lock_handle.close()
            raise ValueError("This data directory is already open in another Relay process. Use a different --data-dir.") from None
        self.lock = threading.RLock()
        self.cancel_event = threading.Event()
        self.active = None
        self.provider_factory = provider_factory
        for path in self.root.glob("*/run.json"):
            run = json.loads(path.read_text())
            if run["status"] not in TERMINAL:
                run.update(status="interrupted", ended_at=now(), error="Server stopped before this run finished.")
                dump(path, run)

    def close(self):
        if self.active:
            raise RuntimeError("Stop the active run before closing the engine.")
        self._lock_handle.close()

    def _dir(self, run_id):
        if not isinstance(run_id, str) or len(run_id) != 12 or any(c not in "0123456789abcdef" for c in run_id):
            raise ValueError("Invalid run ID.")
        path = self.root / run_id
        if not (path / "run.json").is_file():
            raise FileNotFoundError("Run not found.")
        return path

    def list_runs(self):
        with self.lock:
            runs = [json.loads(p.read_text()) for p in self.root.glob("*/run.json")]
        return sorted(runs, key=lambda r: r["created_at"], reverse=True)

    def get(self, run_id):
        with self.lock:
            path = self._dir(run_id)
            run = json.loads((path / "run.json").read_text())
            run["events"] = [json.loads(line) for line in (path / "events.jsonl").read_text().splitlines()]
            run["files"] = snapshot(path / "workspace")
            run["task"] = json.loads((path / "task.json").read_text())
            run["checkpoints"] = [int(p.stem) for p in sorted((path / "checkpoints").glob("*.json"))]
            return run

    def cancel(self, run_id):
        with self.lock:
            self._dir(run_id)
            if self.active != run_id:
                raise ValueError("This run is no longer active.")
            self.cancel_event.set()

    def _limits(self, provided):
        if not isinstance(provided, dict) or set(provided) - set(RANGES):
            raise ValueError("Unknown run limit.")
        result = {**DEFAULT_LIMITS, **provided}
        for key, value in result.items():
            lo, hi = RANGES[key]
            if type(value) is not int or not lo <= value <= hi:
                raise ValueError(f"{key} must be an integer between {lo} and {hi}.")
        return result

    def start(self, config, background=True):
        if not isinstance(config, dict):
            raise ValueError("Run configuration must be an object.")
        with self.lock:
            if self.active:
                raise ValueError("A run is already active. Wait for it or stop it first.")
            parent, checkpoint = config.get("parent_id"), None
            if parent:
                parent_dir = self._dir(parent)
                parent_run = json.loads((parent_dir / "run.json").read_text())
                step = config.get("checkpoint", 0)
                if type(step) is not int or not 0 <= step <= 1000:
                    raise ValueError("Invalid checkpoint.")
                checkpoint_path = parent_dir / "checkpoints" / f"{step:04}.json"
                if not checkpoint_path.is_file():
                    raise ValueError("Checkpoint not found.")
                checkpoint = json.loads(checkpoint_path.read_text())
                task = json.loads((parent_dir / "task.json").read_text())
                provider_name = parent_run["provider"]
                model = config.get("model") or parent_run["model"]
            else:
                task_id = config.get("task_id", "revenue-audit")
                if "task" in config:
                    task = validate_task(config["task"])
                    task.pop("demo", None)
                    task["id"] = "custom"
                elif task_id in TASKS:
                    task = copy.deepcopy(TASKS[task_id])
                else:
                    raise ValueError("Unknown task.")
                provider_name = config.get("provider", "demo")
                model = config.get("model")
            if provider_name not in {"demo", "minimax"}:
                raise ValueError("Choose the demo or MiniMax provider.")
            if provider_name == "demo" and not task.get("demo"):
                raise ValueError("Custom tasks require MiniMax.")
            if model is not None and (not isinstance(model, str) or not model.strip() or len(model) > 120):
                raise ValueError("Invalid model name.")
            additional = config.get("instruction", "")
            if not isinstance(additional, str) or len(additional) > 12000:
                raise ValueError("Additional instruction must be text up to 12,000 characters.")
            if additional and provider_name == "demo":
                raise ValueError("The scripted demo cannot follow new instructions. Use MiniMax.")
            limits = self._limits(config.get("limits", {}))
            provider = self.provider_factory(provider_name, model) if self.provider_factory else (
                DemoProvider() if provider_name == "demo" else MiniMaxProvider(model))
            if isinstance(provider, MiniMaxProvider) and not provider.key and not self.provider_factory:
                raise ValueError("Configure ANTHROPIC_AUTH_TOKEN in .env, then restart Relay.")
            run_id = uuid.uuid4().hex[:12]
            path = self.root / run_id
            (path / "workspace").mkdir(parents=True)
            (path / "checkpoints").mkdir()
            for name, content in (checkpoint["files"] if checkpoint else task["files"]).items():
                write_file(path / "workspace", name, content)
            messages = copy.deepcopy(checkpoint["messages"]) if checkpoint else [
                {"role": "user", "content": task["prompt"]}]
            if additional:
                # Add to the last user turn, preserving tool_result ordering for Anthropic.
                if messages[-1]["role"] == "user":
                    content = messages[-1]["content"]
                    if isinstance(content, str):
                        messages[-1]["content"] += "\n\nAdditional instruction: " + additional
                    else:
                        content.append({"type": "text", "text": "Additional instruction: " + additional})
                else:
                    messages.append({"role": "user", "content": additional})
            run = {"id": run_id, "title": task["title"], "task_id": task.get("id", "custom"),
                   "provider": provider_name, "model": provider.model, "status": "queued", "created_at": now(),
                   "steps": 0, "tool_calls": 0, "input_tokens": 0, "output_tokens": 0,
                   "cache_read_tokens": 0, "cache_creation_tokens": 0,
                   "limits": limits, "checks": [], "parent_id": parent,
                   "parent_checkpoint": config.get("checkpoint", 0) if parent else None,
                   "instruction": additional, "duration_ms": 0, "final": ""}
            dump(path / "task.json", task)
            dump(path / "run.json", run)
            (path / "events.jsonl").touch()
            self._checkpoint(path, 0, messages)
            self.active = run_id
            self.cancel_event = threading.Event()
            cancel = self.cancel_event
        if background:
            threading.Thread(target=self._work, args=(run, task, provider, messages, cancel), daemon=True).start()
        else:
            self._work(run, task, provider, messages, cancel)
        return run_id

    def _checkpoint(self, path, step, messages):
        dump(path / "checkpoints" / f"{step:04}.json", {"messages": messages, "files": snapshot(path / "workspace")})

    def _work(self, run, task, provider, messages, cancel):
        path = self.root / run["id"]
        started = time.monotonic()
        deadline = started + run["limits"]["max_seconds"]
        sequence = 0

        def emit(kind, title, **data):
            nonlocal sequence
            sequence += 1
            run["duration_ms"] = round((time.monotonic() - started) * 1000)
            event = {"seq": sequence, "type": kind, "title": title, "time": now(),
                     "elapsed_ms": run["duration_ms"], **data}
            with self.lock:
                with (path / "events.jsonl").open("a", encoding="utf-8") as handle:
                    handle.write(json.dumps(event, ensure_ascii=False, allow_nan=False) + "\n")
                dump(path / "run.json", run)

        def guard():
            if cancel.is_set():
                raise StopRun("cancelled", "Stopped by user. No further tool actions will run.")
            if time.monotonic() >= deadline:
                raise StopRun("limit", "Run time limit reached.")

        try:
            run.update(status="running", started_at=now())
            emit("start", "Run started", provider=run["provider"], model=run["model"], limits=run["limits"])
            if run["parent_id"]:
                emit("checkpoint", "Restored checkpoint", parent_id=run["parent_id"],
                     checkpoint=run["parent_checkpoint"])
            while run["steps"] < run["limits"]["max_steps"]:
                guard()
                remaining = run["limits"]["total_output_tokens"] - run["output_tokens"]
                if remaining < 64:
                    raise StopRun("limit", "Output token budget exhausted.")
                allowance = min(run["limits"]["max_output_tokens"], remaining)
                run["steps"] += 1
                emit("request", f"Agent turn {run['steps']}", step=run["steps"], max_output_tokens=allowance)
                result_queue = queue.Queue(maxsize=1)

                def request_once():
                    try:
                        result_queue.put((provider.complete(copy.deepcopy(messages), TOOL_SCHEMAS, allowance,
                                                           max(0.1, min(60, deadline - time.monotonic())), task), None))
                    except Exception as exc:
                        result_queue.put((None, exc))

                threading.Thread(target=request_once, daemon=True).start()
                while True:
                    guard()
                    try:
                        response, error = result_queue.get(timeout=0.1)
                        break
                    except queue.Empty:
                        continue
                guard()
                if error:
                    raise error
                usage = response.get("usage") or {}
                for field, key in [("input_tokens", "input_tokens"), ("output_tokens", "output_tokens"),
                                   ("cache_read_tokens", "cache_read_input_tokens"),
                                   ("cache_creation_tokens", "cache_creation_input_tokens")]:
                    value = usage.get(key, 0)
                    if type(value) is not int or value < 0:
                        raise ProviderError("Provider returned invalid token usage.")
                    run[field] += value
                emit("usage", "Response received", usage=usage, stop_reason=response.get("stop_reason"))
                if response.get("stop_reason") == "max_tokens":
                    raise ProviderError("Model output was truncated. No automatic retry was made; raise the per-turn limit before rerunning.")
                if run["output_tokens"] > run["limits"]["total_output_tokens"]:
                    raise StopRun("limit", "Provider exceeded the requested output token budget.")
                content = response["content"]
                # Full blocks (including MiniMax thinking) are retained only in private checkpoints.
                messages.append({"role": "assistant", "content": content})
                texts = [b["text"] for b in content if b.get("type") == "text" and isinstance(b.get("text"), str)]
                for text in texts:
                    emit("message", "Agent message", text=text)
                calls = [b for b in content if b.get("type") == "tool_use"]
                if not calls:
                    if response.get("stop_reason") != "end_turn" or not texts:
                        raise ProviderError("Provider did not return a final answer or a tool call.")
                    run["final"] = "\n".join(texts)
                    guard()
                    run["status"] = "verifying"
                    emit("verify", "Checking workspace artifacts")
                    run["checks"] = verify(path / "workspace", task["checks"])
                    guard()
                    for check in run["checks"]:
                        emit("check", check["name"], **check)
                    run["status"] = ("passed" if all(c["passed"] for c in run["checks"]) else "failed") if run["checks"] else "unverified"
                    break
                if response.get("stop_reason") != "tool_use":
                    raise ProviderError("Provider returned tool calls with an unexpected stop reason.")
                tool_results = []
                ids = set()
                for call in calls:
                    guard()
                    if not isinstance(call.get("id"), str) or call["id"] in ids:
                        raise ProviderError("Provider returned invalid tool call IDs.")
                    ids.add(call["id"])
                    if run["tool_calls"] >= run["limits"]["max_tools"]:
                        raise StopRun("limit", "Tool call limit reached.")
                    name, arguments = call.get("name"), call.get("input")
                    if not isinstance(name, str):
                        raise ProviderError("Provider returned an invalid tool name.")
                    run["tool_calls"] += 1
                    emit("tool_call", name, call_id=call["id"], arguments=arguments)
                    tool_start = time.monotonic()
                    try:
                        with self.lock:
                            result = execute(path / "workspace", name, arguments)
                        failed = False
                    except (ValueError, OSError, ArithmeticError, SyntaxError) as exc:
                        result, failed = {"error": str(exc)}, True
                    emit("tool_result", name, call_id=call["id"], result=result, is_error=failed,
                         duration_ms=round((time.monotonic() - tool_start) * 1000))
                    tool_results.append({"type": "tool_result", "tool_use_id": call["id"],
                                         "content": json.dumps(result), "is_error": failed})
                messages.append({"role": "user", "content": tool_results})
                with self.lock:
                    self._checkpoint(path, run["steps"], messages)
                emit("checkpoint", "Checkpoint saved", step=run["steps"])
            else:
                raise StopRun("limit", "Agent turn limit reached.")
        except StopRun as exc:
            run.update(status=exc.status, error=exc.message)
            emit("stop", exc.message)
        except Exception as exc:
            message = str(exc) if isinstance(exc, ProviderError) else f"Run failed ({type(exc).__name__})."
            run.update(status="error", error=message)
            emit("error", message)
        finally:
            run["ended_at"] = now()
            emit("finish", "Run finished", status=run["status"])
            with self.lock:
                self.active = None
