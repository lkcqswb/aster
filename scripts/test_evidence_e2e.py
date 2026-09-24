"""Real animated terminal: failure, repair, stale evidence and an exact recheck."""
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
    with tempfile.TemporaryDirectory(prefix="aster-evidence-") as temporary:
        project = Path(temporary) / "project"
        project.mkdir()
        state = Path(temporary) / "sessions"
        child = pexpect.spawn(str(ROOT / "aster"), ["--project", str(project), "--state-dir", str(state),
                              "--demo", "--permissions", "allow"],
                              env=dict(os.environ, TERM="xterm-256color", TERM_PROGRAM="iTerm.app"),
                              encoding="utf-8", codec_errors="replace", dimensions=(42, 132))

        class Screen(pyte.Screen):
            def write_process_input(self, data):
                raise AssertionError("Fullscreen Aster should not query cursor position: " + repr(data))

        screen = Screen(132, 42)
        stream = pyte.Stream(screen)
        output = InlineFrames(stream)
        frames = output.frames
        tail = ""

        def pump():
            nonlocal tail
            try:
                chunk = child.read_nonblocking(524288, timeout=.1)
            except pexpect.TIMEOUT:
                return screen_text(screen)
            tail = (tail + chunk)[-10000:]
            output.feed(chunk)
            return screen_text(screen)

        def wait(predicate, timeout=20):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                text = pump()
                if predicate(text):
                    return
            raise AssertionError("Timed out: " + screen_text(screen))

        def saved():
            files = list(state.glob("*.json"))
            return json.loads(files[0].read_text()) if files else {}

        def close_panel():
            before = len(frames)
            child.send("\x1b")
            wait(lambda text: "Checks beside" not in text and len(frames) >= before + 2
                 and "和弄玉说说" in text)

        try:
            wait(lambda _: len(frames) >= 3, 135)
            child.send("/demo evidence\r")
            wait(lambda _: saved().get("status") == "done")
            repaired = saved()["work"]
            assert [e["passed"] for e in repaired["evidence"]] == [False, True]
            assert len({e["identity"] for e in repaired["evidence"]}) == 1
            child.send("\x1b[15~")
            wait(lambda text: "Checks beside" in text and "Recorded checks passed" in text and "earlier result" in text)
            close_panel()
            child.send("/demo evidence stale\r")
            wait(lambda _: saved().get("status") == "done" and "stale" in saved().get("work", {}).get("goal", ""))
            wait(lambda text: "edits need a fresh check" in text)
            stale = saved()["work"]
            assert stale["revision"] > stale["evidence"][-1]["revision"]
            child.send("/checks\r")
            wait(lambda text: "Checks beside" in text and "rerun after edits" in text)
            close_panel()
            child.send('/check evidence-demo.json {"ready":true}\r')
            wait(lambda _: len(saved().get("work", {}).get("evidence", [])) == 3)
            wait(lambda text: "Recorded checks passed" in text and "edits need a fresh check" not in text)
            final = saved()
            assert final["work"]["evidence"][-1]["revision"] == final["work"]["revision"]
            assert final["work"]["model_requests"] == 0
            assert json.loads((project / "evidence-demo.json").read_text()) == {"ready": True}
            child.send("/checks\r")
            wait(lambda text: "Checks beside" in text and "Original outcome: failed" in text)
            child.setwinsize(24, 80)
            screen.resize(24, 80)
            screen.reset()
            wait(lambda text: "Checks beside" in text)
            child.setwinsize(42, 132)
            screen.resize(42, 132)
            screen.reset()
            wait(lambda text: "Checks beside" in text and "Original outcome: failed" in text)
            close_panel()
            child.send("/quit\r")
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
            preview = ROOT / ".aster/qa/evidence-preview-state"
            preview.mkdir(exist_ok=True)
            final["project"] = str(ROOT)
            path = preview / (final["id"] + ".json")
            path.write_text(json.dumps(final, ensure_ascii=False))
            path.chmod(0o600)
            result = {"failed_result_preserved": True, "same_check_recovered": True,
                      "later_edit_invalidated_pass": True, "exact_manual_recheck_refreshed": True,
                      "checks_panel_and_resize": True, "distinct_frames": len(frames), "exit": 0, "api_calls": 0}
            (ROOT / ".aster/qa/evidence-e2e.json").write_text(json.dumps(result, indent=2))
            print(json.dumps(result, indent=2))
        except Exception:
            if not child.isalive():
                child.close()
            diagnostic = {"screen": screen_text(screen), "tail": tail,
                          "exit": child.exitstatus, "signal": child.signalstatus,
                          "saved": saved(), "frames": len(frames)}
            (ROOT / ".aster/qa/evidence-failure.json").write_text(json.dumps(diagnostic, ensure_ascii=False, indent=2))
            raise
        finally:
            stop_child(child, stream)


if __name__ == "__main__":
    main()
