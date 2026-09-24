"""Real animation and local companion controls: portrait, mouse, decisions and stop."""
import json
import os
from pathlib import Path
import tempfile
import time

import pexpect
import pyte
from terminal_helpers import InlineFrames, screen_text, stop_child

ROOT = Path(__file__).resolve().parents[1]


def main():
    with tempfile.TemporaryDirectory(prefix="aster-actions-") as temporary:
        project = Path(temporary) / "project"
        project.mkdir()
        (project / "notes.txt").write_text("Read this before deciding.\n")
        state = Path(temporary) / "sessions"
        child = pexpect.spawn(str(ROOT / "aster"), ["--project", str(project), "--state-dir", str(state), "--demo"],
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

        def paste(text):
            child.send("\x1b[200~" + text + "\x1b[201~")

        def click(x, y):
            child.send(f"\x1b[<0;{x+1};{y+1}M\x1b[<0;{x+1};{y+1}m")

        def click_text(needle):
            row, text = next((i, t) for i, t in enumerate(screen_text(screen).splitlines()) if needle in t)
            click(text.index(needle) + 1, row)

        def command(text):
            paste(text)
            wait(lambda s: text in "\n".join(s.splitlines()[-5:]))
            child.send("\r")

        def menu():
            child.send("\x1bOP")
            wait(lambda text: "Together with 弄玉" in text and "Local controls" in text)

        def close_to(needle):
            before = len(output.frames)
            child.send("\x1b")
            wait(lambda text: needle in text and len(output.frames) >= before + 2)

        try:
            wait(lambda _: len(output.frames) >= 3, 135)
            paste("Keep my task draft")
            wait(lambda text: "Keep my task draft" in text)
            click(105, 15)
            wait(lambda text: "Together with 弄玉" in text and "Local controls" in text)
            paste("files")
            wait(lambda text: "Find: files" in text and "Read project files together" in text)
            click_text("Read project files together")
            wait(lambda text: "Files beside" in text and "notes.txt" in text)
            close_to("Keep my task draft")
            assert not saved().get("messages")
            child.send("\x15")
            wait(lambda text: "Keep my task draft" not in text)
            command("/demo work")
            wait(lambda text: "弄玉 · a question for you" in text and "1–5 choose" in text)
            menu()
            wait(lambda text: "Back to your decision" in text)
            child.send("\r")
            wait(lambda text: "弄玉 · a question for you" in text and "Together with" not in text)
            child.send("2")
            wait(lambda text: "Allow write_file?" in text and '"ready":false' in text)
            menu()
            paste("stop")
            wait(lambda text: "Find: stop" in text and "Stop the current task" in text)
            child.send("\r")
            wait(lambda _: saved().get("status") == "stopped")
            assert not (project / "companion-demo.json").exists()
            wait(lambda text: "Allow write_file?" not in text and "和弄玉说说" in text)
            command("/run printf 'phase-one\\n'; sleep 30")
            wait(lambda text: "Allow shell?" in text and "sleep 30" in text)
            child.send("y")
            wait(lambda text: "phase-one" in text and "Allow shell?" not in text)
            menu()
            wait(lambda text: "Read command output" in text)
            click_text("Read command output")
            wait(lambda text: "Command output" in text and "phase-one" in text and "running" in text.lower())
            before = len(output.frames)
            wait(lambda _: len(output.frames) >= before + 2)
            menu()
            paste("stop")
            wait(lambda text: "Find: stop" in text and "Stop the current task" in text)
            child.send("\r")
            wait(lambda _: saved().get("status") == "stopped" and saved().get("work", {}).get("command", {}).get("stopped"))
            assert not saved()["work"]["command"]["timed_out"]
            assert saved()["work"]["model_requests"] == 0
            menu()
            paste("stop")
            wait(lambda text: "No matching actions" in text)
            child.send("\x15")
            wait(lambda text: "Read command output" in text)
            child.setwinsize(24, 80)
            screen.resize(24, 80)
            screen.reset()
            wait(lambda text: "Together with" in text and "Read command output" in text)
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
            evidence = {"portrait_opens_actions": True, "mouse_opens_files_and_output": True,
                        "composer_preserved": True, "returns_to_question": True,
                        "stopped_before_write": True, "stopped_running_command": True,
                        "obsolete_stop_removed": True, "distinct_frames": len(output.frames),
                        "resize": True, "api_calls": 0, "exit": 0}
            (ROOT / ".aster/qa/actions-e2e.json").write_text(json.dumps(evidence, indent=2))
            print(json.dumps(evidence, indent=2))
        except Exception:
            (ROOT / ".aster/qa/actions-failure.json").write_text(json.dumps({"screen": screen_text(screen),
                 "frames": len(output.frames), "session": saved()}, ensure_ascii=False, indent=2))
            raise
        finally:
            stop_child(child, output)


if __name__ == "__main__":
    main()
