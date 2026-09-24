"""Local animated project picker: browse, page, preview, attach, and send explicit context."""
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
    with tempfile.TemporaryDirectory(prefix="aster-navigator-") as temporary:
        project = Path(temporary) / "project"
        (project / "src").mkdir(parents=True)
        state = Path(temporary) / "sessions"
        source = "unchanged source line\n" * 8000 + "needle jade\nlast\n"
        (project / "src/big source.txt").write_text(source)
        (project / "notes.txt").write_text("".join(f"note {i}\n" for i in range(1, 101)))
        (project / ".env").write_text("fixture-private-value\n")
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
            wait(lambda screen: text in "\n".join(screen.splitlines()[-5:]))
            child.send("\r")

        try:
            wait(lambda _: len(output.frames) >= 3, 135)
            paste("Explain this")
            wait(lambda text: "Explain this" in text)
            child.send("\x1b[17~")
            wait(lambda text: "Files beside" in text)
            paste("NOTES")
            wait(lambda text: "1 result" in text and "notes.txt" in text)
            child.send("\r")
            wait(lambda text: "Reading · notes.txt" in text and "1: note 1" in text)
            before = len(output.frames)
            wait(lambda _: len(output.frames) >= before + 2)
            child.send("\x1b[C")
            wait(lambda text: "41: note 41" in text)
            child.send("\t")
            wait(lambda text: "Explain this @{notes.txt:41-80}" in text and "Reading · notes.txt" not in text)
            assert not saved().get("messages"), "Browsing submitted a model message"
            child.send("\x15")
            wait(lambda text: "Explain this @{notes.txt:41-80}" not in text)
            command("/find NEEDLE")
            wait(lambda text: "Search beside" in text and "src/big source.txt:8001" in text)
            child.send("\r")
            wait(lambda text: "Reading · src/big source.txt" in text and "8001: needle jade" in text)
            child.send("\t")
            wait(lambda text: "@{src/big source.txt:7997-8002}" in text and "Reading · src/big source.txt" not in text)
            assert not saved().get("messages")
            child.send("\r")
            wait(lambda _: saved().get("status") == "done")
            session = saved()
            assert session["work"]["model_requests"] == 0
            assert session["work"]["context_files"] == ["src/big source.txt:7997-8002"]
            assert "8001: needle jade" in session["messages"][0]["content"]
            assert (project / "src/big source.txt").read_text() == source
            child.send("\x1b[17~")
            wait(lambda text: "Files beside" in text and "notes.txt" in text)
            child.setwinsize(24, 80)
            screen.resize(24, 80)
            screen.reset()
            wait(lambda text: "Files beside" in text)
            child.setwinsize(42, 132)
            screen.resize(42, 132)
            screen.reset()
            wait(lambda text: "Files beside" in text and "notes.txt" in text)
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
            evidence = {"draft_preserved": True, "preview_pagination": True, "case_insensitive_picker": True,
                        "large_file_search_and_preview": True, "explicit_context_sent": True, "files_unchanged": True,
                        "animated_during_preview": True, "distinct_frames": len(output.frames), "resize": True,
                        "api_calls": 0, "exit": 0}
            (ROOT / ".aster/qa/navigator-e2e.json").write_text(json.dumps(evidence, indent=2))
            print(json.dumps(evidence, indent=2))
        except Exception:
            (ROOT / ".aster/qa/navigator-failure.json").write_text(json.dumps({"screen": screen_text(screen),
                 "frames": len(output.frames), "session": saved()}, ensure_ascii=False, indent=2))
            raise
        finally:
            stop_child(child, output)


if __name__ == "__main__":
    main()
