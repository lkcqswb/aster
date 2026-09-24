"""One explicit MiniMax continuation from an oversized-session checkpoint; no retries."""
import argparse
import json
import os
from pathlib import Path
import re
import tempfile
import time
import uuid

import pexpect
import pyte
from test_checkpoint_e2e import seed
from terminal_helpers import screen_text, stop_child

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-live", action="store_true")
    if not parser.parse_args().run_live:
        parser.error("--run-live is required; this consumes provider tokens")
    qa = ROOT / ".aster/qa"
    state = qa / ("checkpoint-live-sessions-" + uuid.uuid4().hex[:8])
    with tempfile.TemporaryDirectory(prefix="aster-checkpoint-live-") as temporary:
        project = Path(temporary).resolve()
        (project / "proof.txt").write_text("checkpoint-486\n")
        (project / "AGENTS.md").write_text("# Fixture\nRead proof.txt before writing. Use only update_plan, read_file, write_file and check_file. Never use shell or edit_file. Write only checkpoint-result.json and verify it with json_equals.\n")
        original = seed(project, state, demo=False)
        child = pexpect.spawn(str(ROOT / "aster"), ["--project", str(project), "--state-dir", str(state),
                              "--resume", original["id"], "--no-live2d"], env=dict(os.environ, TERM="xterm-256color"),
                              encoding="utf-8", codec_errors="replace", dimensions=(42, 132))
        screen = pyte.Screen(132, 42)
        stream = pyte.Stream(screen)

        def pump():
            try:
                stream.feed(child.read_nonblocking(65536, timeout=.1))
            except pexpect.TIMEOUT:
                pass
            return screen_text(screen)

        def saved():
            return json.loads((state / (original["id"] + ".json")).read_text())

        def wait(predicate, timeout=20):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if predicate(pump()):
                    return
            raise AssertionError("Timed out: " + screen_text(screen))

        def command(text):
            child.send("\x1b[200~" + text + "\x1b[201~")
            wait(lambda s: text in "\n".join(s.splitlines()[-5:]))
            child.send("\r")

        approved = False
        try:
            wait(lambda text: "aster" in text)
            command("/compact Keep the original exact JSON requirement. Re-read proof.txt.")
            wait(lambda text: "Context checkpoint" in text)
            compacted = saved()
            assert compacted["checkpoint"]["after_bytes"] < 128000
            assert compacted["input_tokens"] == 0
            child.send("\x1b")
            wait(lambda text: "Context checkpoint" not in text and "和弄玉说说" in text)
            prompt = "Continue the original task from our checkpoint. Read proof.txt again; finish and verify the requested JSON."
            command(prompt)
            wait(lambda _: saved()["status"] == "thinking")
            deadline = time.monotonic() + 190
            result = None
            while time.monotonic() < deadline:
                text = pump()
                assert "Allow shell?" not in text and "Allow edit_file?" not in text, "Unexpected mutation requested"
                if "Allow write_file?" in text and "checkpoint-result.json" in text and "checkpoint-486" in text and re.search(r'"verified":\s*true', text) and not approved:
                    assert not (project / "checkpoint-result.json").exists()
                    child.send("y")
                    approved = True
                candidate = saved()
                if candidate["status"] in ("done", "error", "stopped"):
                    result = candidate
                    break
            assert result and result["status"] == "done", "Continuation did not finish"
            assert approved
            actual = json.loads((project / "checkpoint-result.json").read_text())
            assert actual == {"proof": "checkpoint-486", "verified": True}
            assert (project / "proof.txt").read_text() == "checkpoint-486\n"
            calls = [b for m in result["messages"] if isinstance(m.get("content"), list) for b in m["content"] if b.get("type") == "tool_use"]
            assert any(c["name"] == "read_file" and c["input"]["path"] == "proof.txt" for c in calls)
            assert any(c["name"] == "check_file" and c["input"]["kind"] == "json_equals" for c in calls)
            assert any(e["passed"] for e in result["work"]["evidence"])
            assert all(c["name"] not in ("shell", "edit_file") for c in calls)
            child.send("\x03")
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                try:
                    pump()
                except pexpect.EOF:
                    break
            else:
                raise AssertionError("Quit timed out")
            child.close()
            assert child.exitstatus == 0
            evidence = {"session": result["id"], "state_directory": state.name, "model": result["model"],
                        "model_requests": result["work"]["model_requests"], "input_tokens": result["input_tokens"],
                        "output_tokens": result["output_tokens"], "checkpoint_before_bytes": compacted["checkpoint"]["before_bytes"],
                        "checkpoint_after_bytes": compacted["checkpoint"]["after_bytes"], "original_contract_recovered": True,
                        "current_source_read": True, "write_approved": True, "independent_exact_json": actual,
                        "source_unchanged": True, "exit": 0}
            (qa / "checkpoint-live.json").write_text(json.dumps(evidence, indent=2))
            print(json.dumps(evidence, indent=2))
        except Exception:
            s = saved()
            (qa / "checkpoint-live-failure.json").write_text(json.dumps({"state_directory": state.name,
                "input_tokens": s.get("input_tokens", 0), "output_tokens": s.get("output_tokens", 0),
                "approved": approved, "screen": screen_text(screen)}, ensure_ascii=False, indent=2))
            raise
        finally:
            stop_child(child, stream)


if __name__ == "__main__":
    main()
