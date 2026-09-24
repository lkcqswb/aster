"""Animated decision inspection, source attachment, and stale approval rejection; no API."""
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
    with tempfile.TemporaryDirectory(prefix="aster-decision-review-") as temporary:
        project = Path(temporary) / "project"
        project.mkdir()
        (project / "notes.txt").write_text("Use Chinese for this project.\n")
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

        def command(text):
            paste(text)
            wait(lambda s: text in "\n".join(s.splitlines()[-5:]))
            child.send("\r")

        def function(sequence, title):
            child.send(sequence)
            wait(lambda text: title in text and "Decision still waiting" in text)

        def back(title):
            before = len(output.frames)
            child.send("\x1b")
            wait(lambda text: title in text and (title == "Files beside" or "Decision still waiting" not in text)
                 and len(output.frames) >= before + 2)

        try:
            wait(lambda _: len(output.frames) >= 3, 135)
            command("/demo work")
            wait(lambda text: "弄玉 · a question for you" in text and "1–5 choose" in text)
            paste("中")
            function("\x1b[17~", "Files beside")
            paste("notes")
            wait(lambda text: "1 result" in text and "notes.txt" in text)
            child.send("\r")
            wait(lambda text: "Reading · notes.txt" in text and "Use Chinese" in text)
            before = len(output.frames)
            wait(lambda _: len(output.frames) >= before + 2)
            child.send("\t")
            wait(lambda text: "Which language" in text and "@{notes.txt:1-1}" in text)
            paste("文")
            child.send("\r")
            wait(lambda text: "Allow write_file?" in text and '"ready":false' in text)
            target = project / "companion-demo.json"
            assert not target.exists()
            for sequence, title in [("\x1bOQ", "Working together"), ("\x1bOR", "Review changes"),
                                    ("\x1bOS", "Command output"), ("\x1b[15~", "Checks beside")]:
                function(sequence, title)
                child.send("y")
                before = len(output.frames)
                wait(lambda _: len(output.frames) >= before + 2)
                assert not target.exists(), "Inspection accidentally approved a write"
            back("Allow write_file?")
            child.send("y")
            wait(lambda text: "Allow edit_file?" in text and '+{"greeting"' in text)
            function("\x1b[17~", "Files beside")
            paste("companion-demo")
            wait(lambda text: "1 result" in text and "companion-demo.json" in text)
            child.send("\r")
            wait(lambda text: "Reading · companion-demo.json" in text and '"ready":false' in text)
            back("Files beside")
            back("Allow edit_file?")
            child.send("y")
            wait(lambda _: saved().get("status") == "done")
            assert json.loads(target.read_text()) == {"greeting": "你好", "ready": True}
            assert saved()["work"]["evidence"][-1]["passed"]
            # Repeat, then simulate a concurrent editor while the prepared edit is being inspected.
            child.send("\x15")
            wait(lambda text: "@{notes.txt:1-1}" not in text)
            command("/demo work")
            wait(lambda text: "弄玉 · a question for you" in text and "1–5 choose" in text)
            child.send("2")
            wait(lambda text: "Allow write_file?" in text and '"ready":false' in text)
            child.send("y")
            wait(lambda text: "Allow edit_file?" in text and '+{"greeting"' in text)
            function("\x1b[17~", "Files beside")
            concurrent = '{"greeting":"你好","ready":false,"editor":"kept"}\n'
            target.write_text(concurrent)
            back("Allow edit_file?")
            child.send("y")
            wait(lambda _: saved().get("status") == "done" and saved().get("work", {}).get("evidence"))
            assert target.read_text() == concurrent, "Approval overwrote a newer edit"
            assert not saved()["work"]["evidence"][-1]["passed"]
            assert any("changed" in e["text"].lower() for e in saved()["entries"] if e["role"] == "tool")
            assert saved()["work"]["model_requests"] == 0
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
            evidence = {"question_draft_preserved": True, "attached_without_answering": True,
                        "all_review_panels_during_approval": True, "inspection_cannot_approve": True,
                        "preview_before_edit": True, "concurrent_edit_preserved": True,
                        "stale_approval_rejected": True, "distinct_frames": len(output.frames),
                        "api_calls": 0, "exit": 0}
            (ROOT / ".aster/qa/decision-review-e2e.json").write_text(json.dumps(evidence, indent=2))
            print(json.dumps(evidence, indent=2))
        except Exception:
            (ROOT / ".aster/qa/decision-review-failure.json").write_text(json.dumps({"screen": screen_text(screen),
                "frames": len(output.frames), "session": saved()}, ensure_ascii=False, indent=2))
            raise
        finally:
            stop_child(child, output)


if __name__ == "__main__":
    main()
