"""Real-PTY auto-compact: a small context window forces a checkpoint before a request.

Demo mode writes a scripted summary locally, so this makes no API call. The live
model path is covered by the Rust test with a local mock provider.
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


def main():
    binary = ROOT / "target/release/aster"
    with tempfile.TemporaryDirectory(prefix="aster-compact-") as temp:
        project = Path(temp) / "project"
        project.mkdir()
        state = Path(temp) / "sessions"
        for name in "abc":
            (project / f"{name}.txt").write_text("".join(f"{name}-context-line-{i:06d}-padding.\n" for i in range(400)))
        env = dict(os.environ, TERM="xterm-256color", TERM_PROGRAM="iTerm.app")
        env.pop("ANTHROPIC_BASE_URL", None)
        child = pexpect.spawn(str(binary), ["--project", str(project), "--state-dir", str(state), "--demo",
                                            "--no-live2d", "--context-window", "40000", "--auto-compact", "50"],
                              env=env, encoding="utf-8", codec_errors="replace", dimensions=(40, 120), timeout=2)
        screen = pyte.Screen(120, 40)
        stream = pyte.Stream(screen)
        output = InlineFrames(stream)

        def pump():
            try:
                output.feed(child.read_nonblocking(524288, timeout=.1))
            except pexpect.TIMEOUT:
                pass
            return screen_text(screen)

        def wait(predicate, timeout=15, label=""):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                text = pump()
                if predicate(text):
                    return text
            raise AssertionError(f"Timed out {label}\n" + screen_text(screen))

        def send(text):
            child.send("\x1b[200~" + text + "\x1b[201~")
            time.sleep(.3)
            child.send("\r")

        def saved():
            return json.loads(next(state.glob("*.json")).read_text())

        try:
            wait(lambda t: "● ready" in t, label="start")
            send("first look @a.txt:1-400 @b.txt:1-400 @c.txt:1-400")
            wait(lambda t: "● ready" in t and "离线演示" in t, label="first reply")
            meter = wait(lambda t: "ctx " in t)
            assert saved().get("checkpoint") is None
            send("second look @a.txt:1-400 @b.txt:1-400 @c.txt:1-400")
            wait(lambda _: (saved().get("checkpoint") or {}).get("automatic"), label="checkpoint saved")
            wait(lambda t: "Automatic context checkpoint" in t, label="notice in transcript")
            wait(lambda t: "● ready" in t, label="turn finished")
            session = saved()
            checkpoint = session["checkpoint"]
            assert checkpoint["method"] == "demo", checkpoint["method"]
            assert checkpoint["omitted_messages"] >= 2
            assert session["messages"][0]["content"].startswith("[Aster local checkpoint]")
            archive = state / "archive" / checkpoint["archive"]
            archived = json.loads(archive.read_text())
            assert "first look" in json.dumps(archived["messages"])
            assert archive.stat().st_mode & 0o777 == 0o600
            send("/checkpoint")
            wait(lambda t: "Context checkpoint" in t and "automatic" in t and "Offline demo summary" in t,
                 label="checkpoint panel")
            child.send("\x1b")
            child.send("/quit\r")
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                try:
                    output.feed(child.read_nonblocking(524288, timeout=.1))
                except pexpect.TIMEOUT:
                    pass
                except pexpect.EOF:
                    break
            child.close()
            assert child.exitstatus == 0, child.exitstatus
            evidence = {"automatic_checkpoint": True, "method": checkpoint["method"],
                        "before_kb": checkpoint["before_bytes"] // 1000, "after_kb": checkpoint["after_bytes"] // 1000,
                        "archived_messages": len(archived["messages"]), "archive_private": True,
                        "meter_visible": "ctx " in meter, "api_calls": 0}
            qa = ROOT / ".aster/qa"
            qa.mkdir(parents=True, exist_ok=True)
            (qa / "autocompact-e2e.json").write_text(json.dumps(evidence, indent=2))
            print(json.dumps(evidence, indent=2))
        finally:
            stop_child(child, stream)


if __name__ == "__main__":
    main()
