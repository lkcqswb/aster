"""Search and revisit a large visible transcript while the real companion animates."""
import json
import os
from pathlib import Path
import tempfile
import time

import pexpect
import pyte
from test_checkpoint_e2e import seed
from terminal_helpers import InlineFrames, screen_text, stop_child

ROOT = Path(__file__).resolve().parents[1]


def main():
    with tempfile.TemporaryDirectory(prefix="aster-history-") as temporary:
        project = Path(temporary).resolve() / "project"
        project.mkdir()
        state = Path(temporary) / "sessions"
        original = seed(project, state)
        original["entries"] = [{"role": "you" if i % 2 == 0 else "nongyu",
                                "text": f"record-{i:04d}: " + ("An earlier project discussion with 中文 and recorded details. " * 8)} for i in range(1200)]
        original["entries"][486]["text"] = "jade-history-486 · Keep the answer in 中文 and preserve the original file."
        (state / (original["id"] + ".json")).write_text(json.dumps(original))
        child = pexpect.spawn(str(ROOT / "aster"), ["--project", str(project), "--state-dir", str(state), "--resume", original["id"]],
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

        def paste(text):
            child.send("\x1b[200~" + text + "\x1b[201~")

        def history(query):
            child.send("\x1b[18~")
            wait(lambda text: "History beside" in text and "1200 matches" in text)
            paste(query)

        try:
            wait(lambda _: len(output.frames) >= 3, 135)
            wait(lambda text: "record-1199" in text)
            paste("Keep my next request")
            wait(lambda text: "Keep my next request" in text)
            started = time.monotonic()
            history("you: JADE-HISTORY-486")
            wait(lambda text: "1 match" in text and "jade-history-486" in text)
            search_ms = round((time.monotonic() - started) * 1000)
            assert search_ms < 5000
            child.send("\r")
            wait(lambda text: "Conversation #487" in text and "preserve the original" in text)
            before = len(output.frames)
            wait(lambda _: len(output.frames) >= before + 2)
            child.send("\t")
            wait(lambda text: "Conversation #487" not in text and "jade-history-486" in text
                 and "Keep my next request" in text and "Ctrl+End" in text)
            child.send("\x1b[1;5F")
            wait(lambda text: "record-1199" in text and "jade-history-486" not in text)
            history("private fixture reasoning")
            wait(lambda text: "No matching conversation entries" in text)
            child.send("\x1b")
            before = len(output.frames)
            wait(lambda text: "History beside" not in text and len(output.frames) >= before + 2)
            assert "Keep my next request" in screen_text(screen)
            child.send("\x15")
            wait(lambda text: "Keep my next request" not in text)
            paste("/demo work")
            wait(lambda text: "/demo work" in "\n".join(text.splitlines()[-5:]))
            child.send("\r")
            wait(lambda text: "弄玉 · a question for you" in text and "1–5 choose" in text)
            child.send("\x1b[18~")
            wait(lambda text: "History beside" in text and "Decision still waiting" in text)
            paste("jade-history-486")
            wait(lambda text: "1 match" in text)
            child.send("\r")
            wait(lambda text: "Conversation #487" in text)
            child.send("\t")
            wait(lambda text: "decision is still pending" in text and "Conversation #487" in text)
            child.send("\x1b")
            wait(lambda text: "History beside" in text)
            child.send("\x1b")
            wait(lambda text: "弄玉 · a question for you" in text and "Decision still waiting" not in text)
            assert not (project / "companion-demo.json").exists()
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
            saved = json.loads((state / (original["id"] + ".json")).read_text())
            assert saved["work"]["model_requests"] == 0
            evidence = {"transcript_entries": 1200, "history_open_and_search_ms": search_ms,
                        "role_and_case_filter": True, "full_entry_preview": True, "jump_and_return_latest": True,
                        "composer_preserved": True, "provider_thinking_excluded": True,
                        "pending_question_preserved": True, "distinct_frames": len(output.frames), "api_calls": 0, "exit": 0}
            (ROOT / ".aster/qa/history-e2e.json").write_text(json.dumps(evidence, indent=2))
            print(json.dumps(evidence, indent=2))
        except Exception:
            (ROOT / ".aster/qa/history-failure.json").write_text(json.dumps({"screen": screen_text(screen), "frames": len(output.frames)}, ensure_ascii=False, indent=2))
            raise
        finally:
            stop_child(child, output)


if __name__ == "__main__":
    main()
