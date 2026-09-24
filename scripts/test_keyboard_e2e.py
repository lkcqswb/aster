"""Real-PTY keyboard, scrolling and session-control regression. Demo mode; no API calls.

Before 0.18, ↑/↓ in the composer silently scrolled the transcript without a limit,
so a few presses in a short conversation later pinned the view above new replies.
"""
import json
import os
from pathlib import Path
import tempfile
import time

import pexpect
import pyte
from terminal_helpers import InlineFrames, screen_text, stop_child

ROOT = Path(__file__).resolve().parents[1]
UP, DOWN, PGUP = "\x1b[A", "\x1b[B", "\x1b[5~"


def main():
    binary = ROOT / "target/release/aster"
    with tempfile.TemporaryDirectory(prefix="aster-keys-") as temp:
        project = Path(temp) / "project"
        project.mkdir()
        state = Path(temp) / "sessions"
        env = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor", TERM_PROGRAM="iTerm.app")
        env.pop("ANTHROPIC_BASE_URL", None)
        child = pexpect.spawn(str(binary), ["--project", str(project), "--state-dir", str(state), "--demo",
                                            "--no-live2d"], env=env, encoding="utf-8", codec_errors="replace",
                              dimensions=(30, 110), timeout=2)
        screen = pyte.Screen(110, 30)
        stream = pyte.Stream(screen)
        output = InlineFrames(stream)

        def pump():
            try:
                output.feed(child.read_nonblocking(524288, timeout=.1))
            except pexpect.TIMEOUT:
                pass
            return screen_text(screen)

        def wait(predicate, timeout=12, label=""):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                text = pump()
                if predicate(text):
                    return text
            raise AssertionError(f"Timed out {label}\n" + screen_text(screen))

        def composer():
            for line in screen_text(screen).splitlines()[-5:]:
                stripped = line.lstrip(" │")
                if stripped.startswith("›"):
                    return stripped[1:].rstrip(" │")
            return ""

        def send(text):
            child.send("\x1b[200~" + text + "\x1b[201~")
            wait(lambda _: text in composer(), label="paste " + text)
            child.send("\r")
            wait(lambda _: text not in composer(), label="submit " + text)

        try:
            wait(lambda t: "● ready" in t, label="start")
            # Arrow keys in an empty conversation must not move anything.
            for _ in range(8):
                child.send(UP)
            child.send(PGUP)
            time.sleep(.3)
            assert "reading" not in pump()
            for n in range(6):
                send(f"note number {n} for the keyboard test")
                wait(lambda t: "● ready" in t and "离线演示" in t, label=f"reply {n}")
            wait(lambda t: "note number 5" in t)
            # ↑ recalls the previous request instead of scrolling.
            child.send(UP)
            wait(lambda _: "note number 5 for the keyboard test" in composer(), label="history")
            child.send(UP)
            wait(lambda _: "note number 4 for the keyboard test" in composer(), label="older history")
            child.send(DOWN)
            child.send(DOWN)
            wait(lambda _: composer().strip() == "和弄玉说说，你想做什么？", label="draft restored")
            # PageUp reads earlier lines; Esc returns to the latest reply.
            child.send(PGUP)
            text = wait(lambda t: "newer lines" in t, label="scrolled")
            assert "reading" in text
            child.send("\x1b")
            wait(lambda t: "newer lines" not in t and "note number 5" in t, label="back to latest")
            # Option+←/B/F move by word and never type letters into the draft.
            child.send("\x1b[200~alpha beta\x1b[201~")
            wait(lambda _: "alpha beta" in composer())
            child.send("\x1bb")
            child.send("\x1b[1;3D")
            child.send("\x1bf")
            time.sleep(.3)
            assert composer().strip() == "alpha beta", composer()
            # Esc Esc clears the draft; ↑ brings it back.
            child.send("\x1b")
            wait(lambda t: "Esc again clears the draft" in t)
            child.send("\x1b")
            wait(lambda _: "alpha beta" not in composer())
            child.send(UP)
            wait(lambda _: "alpha beta" in composer(), label="cleared draft recalled")
            child.send("\x15")  # Ctrl+U
            wait(lambda _: "alpha beta" not in composer())
            # Session control: a new conversation, then Ctrl+P back to the earlier one.
            send("/rename Keyboard original")
            wait(lambda t: "Keyboard original" in t)
            send("/new Keyboard second")
            wait(lambda t: "Keyboard second" in t and "note number 5" not in t)
            child.send("\x10")  # Ctrl+P
            text = wait(lambda t: "Your conversations" in t and "● current" in t)
            assert "Keyboard original" in text
            child.send("\r")  # default selection is the other conversation
            wait(lambda t: "Your conversations" not in t and "note number 5" in t
                 and "Keyboard original" in t, label="resumed")
            child.send("/quit\r")
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                try:
                    output.feed(child.read_nonblocking(524288, timeout=.1))
                except pexpect.TIMEOUT:
                    pass
                except pexpect.EOF:
                    break
            else:
                raise AssertionError("Aster did not exit")
            child.close()
            assert child.exitstatus == 0, child.exitstatus
            sessions = [json.loads(p.read_text()) for p in state.glob("*.json")]
            assert sorted(s["title"] for s in sessions) == ["Keyboard original", "Keyboard second"]
            evidence = {"history_recall": True, "draft_restored": True, "bounded_scroll": True,
                        "esc_returns_to_latest": True, "option_arrows_type_nothing": True,
                        "esc_esc_clears_recoverably": True, "session_switch": True, "exit": child.exitstatus,
                        "api_calls": 0}
            qa = ROOT / ".aster/qa"
            qa.mkdir(parents=True, exist_ok=True)
            (qa / "keyboard-e2e.json").write_text(json.dumps(evidence, indent=2))
            print(json.dumps(evidence, indent=2))
        finally:
            stop_child(child, stream)


if __name__ == "__main__":
    main()
