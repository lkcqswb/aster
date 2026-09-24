"""Real PTY regression: failed browser startup, visible error, retry, changing frames.

Uses only the local model and an isolated browser. No model API calls.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import tempfile
import time

import pexpect
import pyte

ROOT = Path(__file__).resolve().parents[1]
CHROME = os.environ.get("ASTER_CHROME", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")


def main():
    with tempfile.TemporaryDirectory(prefix="aster-startup-test-") as temporary:
        temp = Path(temporary)
        marker = temp / "failed-once"
        wrapper = temp / "chrome"
        wrapper.write_text(
            "#!/bin/sh\n"
            f"if [ ! -e {shlex.quote(str(marker))} ]; then\n"
            f"  touch {shlex.quote(str(marker))}\n"
            "  echo 'simulated renderer startup failure' >&2\n  exit 7\nfi\n"
            f"exec {shlex.quote(CHROME)} \"$@\"\n"
        )
        wrapper.chmod(0o700)
        state = temp / "sessions"
        env = dict(os.environ, TERM="xterm-256color", TERM_PROGRAM="iTerm.app")
        child = pexpect.spawn(
            str(ROOT / "aster"),
            ["--project", str(temp), "--state-dir", str(state), "--chrome", str(wrapper), "--demo"],
            env=env, encoding="utf-8", codec_errors="replace", dimensions=(42, 132),
        )

        class Screen(pyte.Screen):
            def write_process_input(self, data):
                child.send(data)

        screen = Screen(132, 42)
        stream = pyte.Stream(screen)
        image_buffer = ""
        frames = set()

        def pump():
            nonlocal image_buffer
            try:
                chunk = child.read_nonblocking(524288, timeout=0.1)
            except pexpect.TIMEOUT:
                return
            stream.feed(chunk)
            image_buffer += chunk
            while (match := re.search(r"\x1b\]1337;File=[^:]*:([A-Za-z0-9+/=]+)\x07", image_buffer)):
                frames.add(hashlib.sha256(match[1].encode()).hexdigest())
                image_buffer = image_buffer[match.end():]
            if len(image_buffer) > 4_000_000:
                raise AssertionError("Unterminated native image packet")

        def wait(predicate, timeout=15):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                pump()
                if predicate():
                    return
            raise AssertionError("Timed out:\n" + "\n".join(screen.display))

        try:
            wait(lambda: "/pet retry" in "\n".join(screen.display))
            wait(lambda: "simulated renderer startup failure" in (state / "diagnostics/live2d.json").read_text())
            failure = json.loads((state / "diagnostics/live2d.json").read_text())
            assert "simulated renderer startup failure" in failure["status"], failure
            assert failure["frames"] == 0
            assert failure["info"]["failed_phase"] == "Starting the Live2D renderer…"
            child.send("/pet retry\r")
            wait(lambda: len(frames) >= 4, timeout=65)
            connected = json.loads((state / "diagnostics/live2d.json").read_text())
            assert connected["status"] == "Live2D · connected"
            assert connected["info"]["textures"] == 9
            child.send("/quit\r")
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                try:
                    pump()
                except pexpect.EOF:
                    break
            else:
                raise AssertionError("Renderer did not shut down")
            child.close()
            assert child.exitstatus == 0
            evidence = {
                "startup_failure_visible": True, "diagnostics_saved": True,
                "retry_recovered": True, "distinct_inline_frames": len(frames),
                "textures": 9, "exit": child.exitstatus, "api_calls": 0,
            }
            qa = ROOT / ".aster/qa"
            qa.mkdir(parents=True, exist_ok=True)
            (qa / "live2d-startup-recovery.json").write_text(json.dumps(evidence, indent=2))
            print(json.dumps(evidence, indent=2))
        finally:
            if child.isalive():
                child.terminate(force=True)


if __name__ == "__main__":
    main()
