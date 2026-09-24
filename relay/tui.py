"""Relay's keyboard-first terminal workbench."""
import asyncio
import json
import os
import re
import time
from datetime import datetime
from pathlib import Path

from rich.panel import Panel
from rich.syntax import Syntax
from rich.text import Text
from textual import on
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.screen import ModalScreen
from textual.theme import Theme
from textual.widgets import Button, Footer, Input, Label, OptionList, RichLog, Select, Static, TabbedContent, TabPane, TextArea
from textual.widgets.option_list import Option

from .companion import DEFAULT_PORTRAIT, portrait
from .engine import TERMINAL

SAGE = "#b7cf91"
INK = "#e0e5d9"
MUTED = "#859084"
ROSE = "#d69a90"
GOLD = "#d8c28b"
STATUS_COLORS = {"passed": SAGE, "failed": ROSE, "error": ROSE, "running": GOLD,
                 "queued": GOLD, "verifying": GOLD}


def clean(value):
    """Treat model/file strings as text, never terminal control sequences or markup."""
    value = re.sub(r"\x1b\][^\x07]*(?:\x07|\x1b\\)", "", str(value))
    value = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", value)
    return "".join(c for c in value if c in "\n\t" or 32 <= ord(c) < 127 or ord(c) >= 160)


def pretty(value):
    return clean(json.dumps(value, ensure_ascii=False, indent=2))


class NewRun(ModalScreen):
    BINDINGS = [Binding("escape", "dismiss(None)", "Close"), Binding("ctrl+enter", "submit", "Run")]

    def __init__(self, tasks, config, default_provider=None):
        super().__init__()
        self.tasks, self.config = tasks, config
        self.default_provider = default_provider or ("minimax" if config["minimax_ready"] else "demo")

    def compose(self) -> ComposeResult:
        with Vertical(id="dialog", classes="run-dialog"):
            yield Static("A NEW EXPERIMENT", classes="eyebrow")
            yield Static("Give your agent a task.", classes="modal-title")
            with VerticalScroll(id="form-scroll"):
                yield Label("Task")
                yield Select([(t["title"], t["id"]) for t in self.tasks] + [("Custom task / manifest", "custom")],
                             value="config-repair", allow_blank=False, id="task-choice")
                yield Static("", id="task-hint", classes="hint")
                with Horizontal(classes="field-row"):
                    with Vertical(classes="field"):
                        yield Label("Agent")
                        yield Select([("MiniMax · live", "minimax"), ("Scripted demo", "demo")],
                                     value=self.default_provider, allow_blank=False, id="provider-choice")
                    with Vertical(classes="field"):
                        yield Label("Model")
                        yield Input(self.config["model"], id="model-name")
                with Vertical(id="custom-fields"):
                    yield Label("Task manifest · optional .json path")
                    yield Input(placeholder="examples/custom-task.json", id="manifest-path")
                    yield Label("Instruction · used if no manifest, or overrides its prompt")
                    yield TextArea(id="custom-prompt", soft_wrap=True)
                    yield Static("A manifest supplies starting files and checks. A prompt alone is unverified.", classes="hint")
                yield Static("BOUND THE WORK", classes="eyebrow")
                with Horizontal(classes="field-row"):
                    with Vertical(classes="field"):
                        yield Label("Max turns")
                        yield Input("8", id="turn-limit", type="integer")
                    with Vertical(classes="field"):
                        yield Label("Max seconds")
                        yield Input("120", id="time-limit", type="integer")
                with Horizontal(classes="field-row"):
                    with Vertical(classes="field"):
                        yield Label("Output tokens / turn")
                        yield Input("2048", id="output-limit", type="integer")
                    with Vertical(classes="field"):
                        yield Label("Output tokens / run")
                        yield Input("8000", id="total-limit", type="integer")
                yield Static("", id="provider-hint", classes="hint")
                yield Static("", id="form-error", classes="error-text")
            with Horizontal(classes="dialog-actions"):
                yield Button("Cancel", id="close-new")
                yield Button("Run task  ↗", id="launch", variant="primary")

    def on_mount(self):
        self.update_form()

    @on(Select.Changed)
    def update_form(self):
        if not self.is_mounted:
            return
        task_id = self.query_one("#task-choice", Select).value
        self.query_one("#custom-fields").display = task_id == "custom"
        task = next((t for t in self.tasks if t["id"] == task_id), None)
        self.query_one("#task-hint", Static).update(clean(task["description"]) if task else "Bring your own instruction and success criteria.")
        demo = self.query_one("#provider-choice", Select).value == "demo"
        self.query_one("#model-name", Input).disabled = demo
        self.query_one("#provider-hint", Static).update("Scripted demo · no API usage. Sample tasks only." if demo else
            "MiniMax connected · this run makes API calls." if self.config["minimax_ready"] else "MiniMax key is missing. Configure .env and restart.")
        self.query_one("#launch", Button).disabled = not demo and not self.config["minimax_ready"]

    @on(Button.Pressed, "#close-new")
    def close(self): self.dismiss(None)

    @on(Button.Pressed, "#launch")
    def action_submit(self):
        try:
            provider = str(self.query_one("#provider-choice", Select).value)
            task_id = str(self.query_one("#task-choice", Select).value)
            limits = {"max_steps": int(self.query_one("#turn-limit", Input).value),
                      "max_seconds": int(self.query_one("#time-limit", Input).value),
                      "max_output_tokens": int(self.query_one("#output-limit", Input).value),
                      "total_output_tokens": int(self.query_one("#total-limit", Input).value)}
            config = {"task_id": task_id, "provider": provider, "limits": limits}
            if provider == "minimax": config["model"] = self.query_one("#model-name", Input).value.strip()
            if task_id == "custom":
                if provider == "demo": raise ValueError("Custom tasks require a live model.")
                path = self.query_one("#manifest-path", Input).value.strip()
                prompt = self.query_one("#custom-prompt", TextArea).text.strip()
                if path:
                    file = Path(path).expanduser()
                    if file.suffix != ".json" or file.stat().st_size > 1_200_000:
                        raise ValueError("Choose a JSON manifest smaller than 1.2 MB.")
                    config["task"] = json.loads(file.read_text())
                    if prompt: config["task"]["prompt"] = prompt
                else:
                    config["task"] = {"title": prompt.splitlines()[0][:70] if prompt else "Custom task",
                                      "prompt": prompt, "files": {}, "checks": []}
            # Validate before closing so errors stay beside the user's input.
            from .engine import Engine
            from .tasks import validate_task
            Engine._limits(None, limits)
            if "task" in config: validate_task(config["task"])
            self.dismiss(config)
        except (ValueError, OSError, TypeError) as exc:
            self.query_one("#form-error", Static).update(clean(exc))


class BranchRun(ModalScreen):
    BINDINGS = [Binding("escape", "dismiss(None)", "Close")]

    def __init__(self, run):
        super().__init__()
        self.run = run

    def compose(self):
        with Vertical(id="dialog"):
            yield Static("TRY ANOTHER PATH", classes="eyebrow")
            yield Static("Branch from a checkpoint.", classes="modal-title")
            with VerticalScroll(id="form-scroll"):
                yield Static("The original run is preserved. This branch starts with fresh limits.", classes="hint")
                yield Label("Resume from")
                yield Select([("Beginning of this run" if n == 0 else f"After turn {n}", n)
                              for n in self.run["checkpoints"]], value=0, allow_blank=False, id="checkpoint")
                yield Label("Additional instruction")
                yield TextArea(id="branch-instruction", disabled=self.run["provider"] == "demo")
                yield Label("Model")
                yield Input(self.run["model"], id="branch-model", disabled=self.run["provider"] == "demo")
                yield Static("MiniMax branches make new API calls; the original checks stay fixed." if self.run["provider"] == "minimax"
                             else "Demo replay follows the same script; it does not interpret new instructions.", classes="hint")
            with Horizontal(classes="dialog-actions"):
                yield Button("Cancel", id="close-branch")
                yield Button("Start branch  ↗", id="start-branch", variant="primary")

    @on(Button.Pressed, "#close-branch")
    def close(self): self.dismiss(None)

    @on(Button.Pressed, "#start-branch")
    def submit(self):
        self.dismiss({"parent_id": self.run["id"], "checkpoint": self.query_one("#checkpoint", Select).value,
                      "instruction": self.query_one("#branch-instruction", TextArea).text,
                      "model": self.query_one("#branch-model", Input).value, "limits": self.run["limits"]})


class QuitRun(ModalScreen):
    BINDINGS = [Binding("escape", "dismiss(False)", "Keep working")]

    def compose(self):
        with Vertical(id="quit-dialog"):
            yield Static("An agent is still working.", classes="modal-title")
            yield Static("Exiting this local session stops new tool actions. An in-flight model request may still use tokens.", classes="hint")
            with Horizontal(classes="dialog-actions"):
                yield Button("Keep working", id="keep-working", variant="primary")
                yield Button("Stop and exit", id="quit-stop", variant="error")

    @on(Button.Pressed)
    def choose(self, event): self.dismiss(event.button.id == "quit-stop")


class RelayTUI(App):
    TITLE = "Relay"
    SUB_TITLE = "Agent workbench"
    CSS_PATH = "terminal.tcss"
    ENABLE_COMMAND_PALETTE = True
    BINDINGS = [
        Binding("n", "new_run", "New run"), Binding("b", "branch", "Branch"),
        Binding("s", "stop", "Stop"), Binding("e", "export", "Export"),
        Binding("1", "tab('timeline-tab')", "Timeline", show=False),
        Binding("2", "tab('checks-tab')", "Checks", show=False),
        Binding("3", "tab('files-tab')", "Files", show=False),
        Binding("4", "tab('task-tab')", "Task", show=False),
        Binding("h", "history", "History", show=False),
        Binding("c", "companion", "Companion", show=False),
        Binding("r", "refresh", "Refresh", show=False),
        Binding("q", "quit", "Quit"), Binding("ctrl+c", "quit", "Quit", show=False, priority=True),
    ]

    def __init__(self, backend, export_dir, companion_path=None, show_companion=True, default_provider=None):
        super().__init__()
        self.backend, self.export_dir = backend, Path(export_dir)
        self.companion_path = companion_path or DEFAULT_PORTRAIT
        self.show_companion = show_companion
        self.default_provider = default_provider
        self.settings = {}; self.tasks = []; self.runs = []; self.current = None
        self.selected = None; self.file_selected = None; self.last_export = None
        self._busy = False; self._rendered = None; self._run_signature = None; self._seen = 0
        self._detail_signature = None
        self._ticks = 0; self._quitting = False; self._history_visible = False

    def compose(self):
        with Horizontal(id="topbar"):
            yield Static("↗  relay", id="brand")
            yield Static("TERMINAL WORKBENCH", id="wordmark")
            yield Static("connecting…", id="connection")
        with Horizontal(id="body"):
            with Vertical(id="sidebar"):
                yield Static("ASTER / WORKSPACE", id="workspace-label")
                yield Static("RUN HISTORY", classes="eyebrow")
                yield OptionList(id="history")
                with Vertical(id="companion-box"):
                    yield Static("COMPANION / 弄玉", classes="eyebrow")
                    yield Static(id="portrait")
                    yield Static("◇  A quiet place to begin.", id="companion-status")
                yield Static("LOCAL FILES · VISIBLE WORK", id="sidebar-foot")
            with Vertical(id="workbench"):
                yield Static("A place for your agent to work.", id="run-heading")
                yield Static("Choose a task. Follow the work. Check the result.", id="run-meta")
                with Horizontal(id="toolbar"):
                    yield Button("＋ New run", id="new")
                    yield Button("↻ Branch", id="branch")
                    yield Button("■ Stop", id="stop", disabled=True)
                    yield Button("↓ Export", id="export")
                with TabbedContent(id="pages", initial="timeline-tab"):
                    with TabPane("01 Timeline", id="timeline-tab"):
                        yield RichLog(id="timeline", wrap=True, highlight=False, markup=False)
                    with TabPane("02 Checks", id="checks-tab"):
                        yield RichLog(id="checks", wrap=True, markup=False)
                    with TabPane("03 Files", id="files-tab"):
                        with Horizontal(id="file-browser"):
                            yield OptionList(id="files")
                            yield RichLog(id="file-content", wrap=True, markup=False)
                    with TabPane("04 Task", id="task-tab"):
                        yield RichLog(id="task-content", wrap=True, markup=False)
                yield Static("NO ACTIVE RUN", id="budget")
                yield Static("n  new run   ·   1–4  switch views   ·   h  history   ·   c  companion", id="notice")
        yield Footer()

    async def on_mount(self):
        self.register_theme(Theme(name="relay", primary=SAGE, secondary="#91a87f", accent=SAGE,
                                  foreground=INK, background="#101713", surface="#151c19", panel="#1c281f",
                                  success=SAGE, warning=GOLD, error=ROSE, dark=True))
        self.theme = "relay"
        self.query_one("#portrait", Static).update(portrait(str(self.companion_path)))
        self.query_one("#companion-box").display = self.show_companion
        try:
            self.settings, self.tasks = await asyncio.to_thread(lambda: (self.backend.config(), self.backend.tasks()))
            label = "● MiniMax ready" if self.settings["minimax_ready"] else "○ Demo ready"
            self.query_one("#connection", Static).update(Text(f"{label}  /  {self.backend.mode}", style=SAGE))
            self.query_one("#timeline", RichLog).write(Panel(
                Text("Every step. In the open.\n\nStart a task with n. The agent can read and write files,\n"
                     "use tools, and leave a complete trace. Relay checks its\noutput against your original success criteria.\n\n"
                     "Live model or scripted demo. Your choice.", style=MUTED),
                title="WELCOME TO RELAY", border_style="#354338", padding=(2, 3)))
            await self.refresh_data()
            self.set_interval(.6, self.tick)
        except Exception as exc:
            self.tell(clean(exc), error=True)
        self.on_resize()

    def on_resize(self, event=None):
        if not self.is_mounted: return
        size = event.size if event else self.size
        compact = size.width < 105
        self.set_class(compact, "compact")
        self.set_class(size.height < 32, "short")
        self.query_one("#sidebar").display = not compact or self._history_visible
        self.query_one("#companion-box").display = self.show_companion and not compact and size.height >= 34

    def tell(self, message, error=False):
        self.query_one("#notice", Static).update(Text(clean(message), style=ROSE if error else SAGE))

    async def tick(self):
        self._ticks += 1
        active = any(r["status"] not in TERMINAL for r in self.runs)
        if active or self._ticks % 8 == 0:
            await self.refresh_data()

    async def refresh_data(self):
        if self._busy or not self.settings or self._quitting: return
        self._busy = True
        try:
            runs = await asyncio.to_thread(self.backend.list_runs)
            self.runs = runs
            signature = [(r["id"], r["status"]) for r in runs]
            if signature != self._run_signature:
                history = self.query_one("#history", OptionList)
                history.clear_options()
                for run in runs:
                    label = Text()
                    label.append("● ", STATUS_COLORS.get(run["status"], MUTED))
                    label.append(clean(run["title"])[:22], INK)
                    label.append(f"\n  {run['id'][:6]} · {run['status']}\n", MUTED)
                    history.add_option(Option(label, id=run["id"]))
                self._run_signature = signature
            if not self.selected and runs: self.selected = runs[0]["id"]
            if self.selected:
                selected = self.selected
                run = await asyncio.to_thread(self.backend.get, selected)
                if selected == self.selected:
                    self.current = run
                    self.render_run()
            busy = any(r["status"] not in TERMINAL for r in runs)
            self.query_one("#new", Button).disabled = busy
            self.query_one("#branch", Button).disabled = busy or not self.current
            self.query_one("#export", Button).disabled = not self.current
            self.query_one("#stop", Button).disabled = not busy
        except Exception as exc:
            self.tell(clean(exc), error=True)
        finally:
            self._busy = False

    def render_run(self):
        run = self.current
        signature = (run["id"], len(run["events"]), run["status"], tuple(run["files"].items()))
        if signature == self._detail_signature:
            return
        self._detail_signature = signature
        color = STATUS_COLORS.get(run["status"], MUTED)
        self.query_one("#run-heading", Static).update(Text(clean(run["title"]), style=f"bold {INK}"))
        meta = Text(f"● {run['status'].upper()}  ", style=color)
        meta.append(f"{run['id'][:6]}  /  {run['model']}  /  {run['duration_ms']/1000:.1f}s", MUTED)
        if run["parent_id"]: meta.append(f"  ↳ {run['parent_id'][:6]}:{run['parent_checkpoint']}", SAGE)
        if run["provider"] == "demo": meta.append("  SCRIPTED", GOLD)
        self.query_one("#run-meta", Static).update(meta)
        log = self.query_one("#timeline", RichLog)
        if self._rendered != run["id"]:
            log.clear(); self._seen = 0; self._rendered = run["id"]; self.file_selected = None
        for event in run["events"][self._seen:]: self.render_event(log, event)
        self._seen = len(run["events"])
        self.render_checks()
        file_list = self.query_one("#files", OptionList)
        names = sorted(run["files"])
        existing = [file_list.get_option_at_index(i).id for i in range(file_list.option_count)]
        if existing != names:
            file_list.clear_options()
            file_list.add_options([Option(Text("▤ " + clean(name), style=SAGE), id=name) for name in names])
        if self.file_selected not in names:
            self.file_selected = next((n for n in names if n not in run["task"]["files"]), names[0] if names else None)
        self.render_file()
        task_log = self.query_one("#task-content", RichLog)
        task_log.clear()
        task_log.write(Text("ORIGINAL TASK\n", style=f"bold {SAGE}"))
        task_log.write(Text(clean(run["task"]["prompt"]), style=INK))
        if run["instruction"]:
            task_log.write(Text("\nBRANCH INSTRUCTION\n" + clean(run["instruction"]), style=SAGE))
        task_log.write(Text("\nFROZEN SUCCESS CRITERIA", style=SAGE))
        task_log.write(Syntax(pretty(run["task"]["checks"]), "json", theme="monokai", background_color="#151c19", word_wrap=True))
        task_log.write(Text(f"\nProvider usage  input {run['input_tokens']:,}  /  output {run['output_tokens']:,}\n"
                            f"cache read {run['cache_read_tokens']:,}  /  cache creation {run['cache_creation_tokens']:,}", style=MUTED))
        limits = run["limits"]
        budget = (f"TURNS {run['steps']}/{limits['max_steps']}   TOOLS {run['tool_calls']}/{limits['max_tools']}   "
                  f"OUTPUT {run['output_tokens']:,}/{limits['total_output_tokens']:,}   TIME {run['duration_ms']/1000:.0f}/{limits['max_seconds']}s")
        self.query_one("#budget", Static).update(Text(budget, style=MUTED))
        status = {"passed": "✓  The checks agree.", "failed": "×  Found a mismatch.", "running": "◌  Following the work…",
                  "cancelled": "◇  Taking a pause.", "error": "!  Something needs a look."}.get(run["status"], "◇  Here with you.")
        self.query_one("#companion-status", Static).update(Text(status, style=color))

    def render_event(self, log, event):
        kind = event["type"]
        if kind == "usage": return
        if kind == "request":
            log.write(Text(f"\n  ──  TURN {event['step']:02}  " + "─" * 30, style="#65745f"))
            return
        icon = {"start":"↗", "tool_call":"⌘", "tool_result":"↳", "checkpoint":"◇", "message":"•",
                "check":"✓" if event.get("passed") else "×", "verify":"◎", "finish":"■", "error":"!", "stop":"■"}.get(kind, "·")
        color = ROSE if kind == "error" or kind == "check" and not event.get("passed") else SAGE if kind in {"tool_call", "check"} else MUTED
        title = Text(f"  {icon}  {clean(event['title'])}", style=color)
        title.append(f"   +{event['elapsed_ms']/1000:.1f}s", style="#5f6b60")
        log.write(title)
        if kind == "tool_call":
            log.write(Panel(Syntax(pretty(event["arguments"]), "json", theme="monokai", background_color="#1b2420", word_wrap=True),
                            border_style="#314335", padding=(0, 2)))
        elif kind == "tool_result":
            content = pretty(event["result"])
            if len(content) > 900: content = content[:900] + "\n… full result in exported trace"
            log.write(Text("     " + content.replace("\n", "\n     "), style=ROSE if event["is_error"] else "#7f9380"))
        elif kind == "message":
            log.write(Text("\n" + clean(event["text"]).strip() + "\n", style=INK))
        elif kind == "finish":
            log.write(Text(f"\n  {event['status'].upper()}  ·  run recorded\n", style=STATUS_COLORS.get(event['status'], MUTED)))

    def render_checks(self):
        run = self.current
        log = self.query_one("#checks", RichLog)
        log.clear()
        if not run["checks"]:
            text = "No success checks were defined. Result is unverified." if run["status"] == "unverified" else (
                "Run ended before verification." if run["status"] in TERMINAL else "Checks run after the agent finishes.")
            log.write(Text(text + "\n", style=MUTED))
            for check in run["task"]["checks"]: log.write(Text("○  " + clean(check.get("name", check["type"])), style=MUTED))
        else:
            passed = sum(c["passed"] for c in run["checks"])
            log.write(Text(f"\n  {passed} / {len(run['checks'])}  CHECKS PASSED\n", style=f"bold {SAGE if passed == len(run['checks']) else ROSE}"))
            for check in run["checks"]:
                color = SAGE if check["passed"] else ROSE
                log.write(Text(f"  {'✓' if check['passed'] else '×'}  {clean(check['name'])}\n     {clean(check['path'])}\n", style=color))
                if not check["passed"]:
                    details = {k: check[k] for k in ("detail", "expected", "actual") if k in check}
                    log.write(Panel(Syntax(pretty(details), "json", theme="monokai", word_wrap=True), border_style="#5a3f38"))
        if run.get("error"): log.write(Text("\n" + clean(run["error"]), style=ROSE))
        log.write(Text("\nVerification runs outside the agent loop.\nPassing establishes these specific checks only.", style=MUTED))

    def render_file(self):
        log = self.query_one("#file-content", RichLog)
        log.clear()
        if not self.file_selected:
            log.write(Text("No files in this workspace yet.", style=MUTED)); return
        name = self.file_selected
        text = self.current["files"][name]
        log.write(Text(f"{clean(name)}  ·  {len(text.encode()):,} bytes\n", style=SAGE))
        language = {".json":"json", ".py":"python", ".md":"markdown", ".csv":"text"}.get(Path(name).suffix, "text")
        log.write(Syntax(clean(text), language, theme="monokai", background_color="#151c19", line_numbers=True, word_wrap=True))

    @on(OptionList.OptionSelected, "#history")
    async def select_history(self, event):
        self.selected = event.option.id
        await self.refresh_data()
        if self.size.width < 105:
            self._history_visible = False; self.on_resize()

    @on(OptionList.OptionSelected, "#files")
    def select_file(self, event):
        self.file_selected = event.option.id; self.render_file()

    @on(Button.Pressed, "#new")
    def action_new_run(self):
        if not self.settings: return
        if any(r["status"] not in TERMINAL for r in self.runs):
            self.tell("An agent is already working. Stop it or wait for completion."); return
        self.push_screen(NewRun(self.tasks, self.settings, self.default_provider), self.launch)

    async def launch(self, config):
        if config is None: return
        try:
            run_id = await asyncio.to_thread(self.backend.start, config)
            self.selected = run_id
            self.query_one("#pages", TabbedContent).active = "timeline-tab"
            self.tell("Agent started · every action will be recorded.")
            await self.refresh_data()
        except Exception as exc:
            self.tell(clean(exc), error=True)

    @on(Button.Pressed, "#branch")
    def action_branch(self):
        if not self.current: return
        if any(r["status"] not in TERMINAL for r in self.runs):
            self.tell("Wait for the active run before branching."); return
        self.push_screen(BranchRun(self.current), self.launch)

    @on(Button.Pressed, "#stop")
    async def action_stop(self):
        active = next((r for r in self.runs if r["status"] not in TERMINAL), None)
        if active:
            try:
                await asyncio.to_thread(self.backend.cancel, active["id"])
                self.tell("Stopping new actions. In-flight API usage may still be billed.")
                await self.refresh_data()
            except Exception as exc: self.tell(clean(exc), error=True)

    @on(Button.Pressed, "#export")
    async def action_export(self):
        if not self.current: return
        try:
            run = await asyncio.to_thread(self.backend.get, self.current["id"])
            self.export_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
            path = self.export_dir / f"relay-{run['id']}-{datetime.now():%Y%m%d-%H%M%S-%f}.json"
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, "w") as handle: json.dump(run, handle, ensure_ascii=False, indent=2)
            self.last_export = path
            self.tell(f"Exported → {path}")
        except Exception as exc: self.tell(clean(exc), error=True)

    def action_tab(self, pane): self.query_one("#pages", TabbedContent).active = pane

    def action_history(self):
        self._history_visible = not self._history_visible if self.size.width < 105 else True
        self.on_resize()
        history = self.query_one("#history", OptionList)
        if history.option_count:
            history.highlighted = next((i for i, run in enumerate(self.runs) if run["id"] == self.selected), 0)
        history.focus()

    def action_companion(self):
        self.show_companion = not self.show_companion; self.on_resize()

    async def action_refresh(self): await self.refresh_data()

    async def action_quit(self):
        if len(self.screen_stack) > 1: return
        active = next((r for r in self.runs if r["status"] not in TERMINAL), None)
        if active and self.backend.owns_engine:
            self.push_screen(QuitRun(), self.finish_quit)
        else:
            self.exit()

    async def finish_quit(self, stop):
        if not stop: return
        await self.action_stop()
        self._quitting = True
        for _ in range(30):
            if not self.backend.engine.active: break
            await asyncio.sleep(.1)
        self.exit()


def run_terminal(backend, export_dir, **kwargs):
    app = RelayTUI(backend, export_dir, **kwargs)
    try:
        app.run()
    finally:
        if backend.owns_engine:
            if backend.engine.active:
                backend.engine.cancel(backend.engine.active)
                deadline = time.monotonic() + 3
                while backend.engine.active and time.monotonic() < deadline: time.sleep(.05)
            if not backend.engine.active: backend.engine.close()
