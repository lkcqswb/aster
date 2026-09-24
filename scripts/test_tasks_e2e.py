"""Local project tasks, reviewed execution, output and honest headless exit statuses."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

import pexpect
import pyte
from terminal_helpers import InlineFrames, screen_text, stop_child

ROOT = Path(__file__).resolve().parents[1]
TASKS = {"tasks": [
    {"name": "proof", "description": "Check a real local fixture", "command": "printf 'START-PROOF\\n'; sleep 2; printf task-proof > proof.txt; printf 'PASS-PROOF\\n'", "timeout_secs": 10},
    {"name": "failure", "description": "Report a real failing exit", "command": "printf 'EXPECTED-FAILURE\\n' >&2; exit 7", "timeout_secs": 5},
    {"name": "denied", "command": "touch never-approved", "timeout_secs": 5}
]}


def main():
    with tempfile.TemporaryDirectory(prefix="aster-tasks-") as temporary:
        project = Path(temporary).resolve() / "project"
        project.mkdir()
        (project / ".aster").mkdir()
        (project / ".aster/tasks.json").write_text(json.dumps(TASKS))
        state = Path(temporary) / "sessions"
        child = pexpect.spawn(str(ROOT / "aster"), ["--project", str(project), "--state-dir", str(state)],
                              env=dict(os.environ, TERM="xterm-256color", TERM_PROGRAM="iTerm.app"),
                              encoding="utf-8", codec_errors="replace", dimensions=(42, 132))
        screen = pyte.Screen(132, 42)
        output = InlineFrames(pyte.Stream(screen))

        def pump():
            try:
                output.feed(child.read_nonblocking(524288, timeout=.1))
            except pexpect.TIMEOUT:
                pass
            return screen_text(screen)

        def wait(predicate, timeout=20):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if predicate(pump()):
                    return
            raise AssertionError("Timed out: " + screen_text(screen))

        def saved():
            files = list(state.glob("*.json"))
            return json.loads(files[0].read_text()) if files else {}

        def command(text):
            child.send("\x1b[200~" + text + "\x1b[201~")
            wait(lambda s: text in "\n".join(s.splitlines()[-5:]))
            child.send("\r")

        def close_panel():
            before = len(output.frames)
            child.send("\x1b")
            wait(lambda text: len(output.frames) >= before + 2 and "和弄玉说说" in text)

        try:
            wait(lambda _: len(output.frames) >= 3, 135)
            child.send("\x1b[19~")  # F8
            wait(lambda text: "Project tasks beside" in text and "proof" in text)
            assert not (project / "proof.txt").exists()
            child.send("\x1b[200~proof\x1b[201~\t")
            wait(lambda text: "Project task · proof" in text and "START-PROOF" in text and "10 seconds" in text)
            child.send("\x1b[200~failure\x1b[201~")
            before = len(output.frames)
            wait(lambda _: len(output.frames) >= before + 2)
            assert "Project task · proof" in screen_text(screen)
            child.send("\r")
            wait(lambda text: "Allow shell?" in text and "START-PROOF" in text)
            assert not (project / "proof.txt").exists()
            child.send("y\x1bOS")  # F4: streamed output
            wait(lambda text: "Command output" in text and "START-PROOF" in text)
            assert not (project / "proof.txt").exists(), "Output must arrive before the task exits"
            wait(lambda _: saved().get("status") == "done")
            assert (project / "proof.txt").read_text() == "task-proof"
            assert saved()["work"]["evidence"][-1]["passed"]
            close_panel()
            command("/task failure")
            wait(lambda text: "Allow shell?" in text and "EXPECTED-FAILURE" in text)
            child.send("y")
            wait(lambda _: saved().get("status") == "done" and saved().get("work", {}).get("command", {}).get("exit_code") == 7)
            wait(lambda text: "a check needs attention" in text)
            child.send("\x1bOS")
            wait(lambda text: "Command output" in text and "EXPECTED-FAILURE" in text)
            assert saved()["work"]["model_requests"] == 0
            assert saved()["input_tokens"] == saved()["output_tokens"] == 0
            child.setwinsize(24, 80)
            screen.resize(24, 80)
            screen.reset()
            wait(lambda text: "Command output" in text)
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
        except Exception:
            (ROOT / ".aster/qa/tasks-failure.json").write_text(json.dumps({"screen": screen_text(screen), "frames": len(output.frames)}, ensure_ascii=False, indent=2))
            raise
        finally:
            stop_child(child, output)

        outcomes = {}
        cases = [("success", "/task proof", "allow", 0), ("failure", "/task failure", "allow", 1),
                 ("denied", "/task denied", "ask", 1), ("unknown", "/task missing", "allow", 1),
                 ("repaired", "evidence demo", "allow", 0), ("stale", "evidence demo stale", "allow", 1),
                 ("failed_check", "demo task", "deny", 1), ("chat", "hello", "ask", 0)]
        for name, prompt, permissions, expected in cases:
            case_state = Path(temporary) / ("headless-" + name)
            args = [str(ROOT / "aster"), "--project", str(project), "--state-dir", str(case_state),
                    "--no-live2d", "--permissions", permissions, "--prompt", prompt]
            if not prompt.startswith("/task "):
                args.append("--demo")
            result = subprocess.run(args, text=True, capture_output=True, timeout=20)
            assert result.returncode == expected, (name, result.returncode, result.stdout, result.stderr)
            session = json.loads(next(case_state.glob("*.json")).read_text())
            assert session["work"]["model_requests"] == 0
            outcomes[name] = result.returncode
        assert not (project / "never-approved").exists()
        evidence = {"inspect_does_not_execute": True, "paste_preserves_inspected_selection": True,
                    "approval_precedes_execution": True, "output_before_exit": True, "file_proof": "task-proof",
                    "failure_status_and_stderr": True, "companion_attention": True,
                    "distinct_frames": len(output.frames), "headless_exit_codes": outcomes, "api_calls": 0, "resize": True, "exit": 0}
        (ROOT / ".aster/qa/tasks-e2e.json").write_text(json.dumps(evidence, indent=2))
        print(json.dumps(evidence, indent=2))


if __name__ == "__main__":
    main()
