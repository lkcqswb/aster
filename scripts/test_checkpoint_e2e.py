"""Checkpoint an oversized session, inspect it beside animation, and restore exact history."""
import json
import os
from pathlib import Path
import tempfile
import time

import pexpect
import pyte
from terminal_helpers import InlineFrames, screen_text, stop_child

ROOT = Path(__file__).resolve().parents[1]


def seed(project, state, demo=True):
    state.mkdir(parents=True, exist_ok=True)
    state.chmod(0o700)
    session_id = "c0ffee123456"
    request = ('Read proof.txt and create checkpoint-result.json with exactly {"proof":"checkpoint-486",'
               '"verified":true}. Do not modify proof.txt. Independently check the saved JSON.')
    messages = [{"role": "user", "content": request},
                {"role": "assistant", "content": [{"type": "thinking", "thinking": "private fixture reasoning",
                  "signature": "fixture-signature"}, {"type": "tool_use", "id": "large-read", "name": "read_file",
                  "input": {"path": "old-log.txt"}}]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "large-read",
                  "content": json.dumps({"content": "old log line\n" * 40000}), "is_error": False}]},
                {"role": "assistant", "content": [{"type": "text", "text": "The earlier log was read; the requested result is still unfinished."}]}]
    session = {"version": 2, "id": session_id, "title": "Long task checkpoint", "project": str(project.resolve()),
               "created": "2026-09-24T00:00:00+00:00", "updated": "2026-09-24T00:00:00+00:00",
               "model": "MiniMax-M2.7", "demo": demo, "mode": "build", "status": "done",
               "entries": [{"role": "you", "text": request}, {"role": "nongyu", "text": "The result is still unfinished."}],
               "messages": messages, "input_tokens": 0, "output_tokens": 0, "tools": 1, "checks": [],
               "parent": None, "pending": [], "work": {"goal": request}}
    path = state / (session_id + ".json")
    path.write_text(json.dumps(session))
    path.chmod(0o600)
    return session


def main():
    with tempfile.TemporaryDirectory(prefix="aster-checkpoint-") as temporary:
        project = Path(temporary).resolve() / "project"
        project.mkdir()
        (project / "proof.txt").write_text("checkpoint-486\n")
        state = Path(temporary) / "sessions"
        original = seed(project, state)
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

        def command(text):
            child.send("\x1b[200~" + text + "\x1b[201~")
            wait(lambda s: text in "\n".join(s.splitlines()[-5:]))
            child.send("\r")

        try:
            wait(lambda _: len(output.frames) >= 3, 135)
            command("/compact Keep the exact JSON contract; re-read proof.txt.")
            wait(lambda text: "Context checkpoint" in text and "messages archived" in text)
            compacted = json.loads((state / (original["id"] + ".json")).read_text())
            checkpoint = compacted["checkpoint"]
            assert checkpoint["before_bytes"] > 400000
            assert checkpoint["after_bytes"] < 128000
            assert checkpoint["kept_messages"] == 0
            assert "checkpoint-486" in checkpoint["summary"]
            assert "re-read proof.txt" in checkpoint["note"]
            assert "private fixture reasoning" not in checkpoint["summary"]
            archived_path = state / "archive" / checkpoint["archive"]
            assert json.loads(archived_path.read_text())["messages"] == original["messages"]
            assert archived_path.stat().st_mode & 0o777 == 0o600
            before = len(output.frames)
            wait(lambda _: len(output.frames) >= before + 2)
            child.send("\x1b")
            wait(lambda text: "Context checkpoint" not in text and "和弄玉说说" in text)
            (project / "proof.txt").write_text("newer file state\n")
            command("/restore " + checkpoint["id"])
            wait(lambda _: len(list(state.glob("*.json"))) == 2)
            restored = next(json.loads(p.read_text()) for p in state.glob("*.json") if p.stem != original["id"])
            assert restored["parent"] == original["id"]
            assert restored["messages"] == original["messages"]
            assert (project / "proof.txt").read_text() == "newer file state\n"
            assert json.loads((state / (original["id"] + ".json")).read_text())["messages"] == compacted["messages"]
            assert restored["work"]["model_requests"] == 0
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
            evidence = {"single_oversized_exchange_compacted": True, "before_bytes": checkpoint["before_bytes"],
                        "after_bytes": checkpoint["after_bytes"], "note_and_original_request_retained": True,
                        "private_archive_exact": True, "restore_forks_without_overwriting_current_session": True,
                        "project_files_unchanged_by_restore": True, "distinct_frames": len(output.frames), "api_calls": 0, "exit": 0}
            (ROOT / ".aster/qa/checkpoint-e2e.json").write_text(json.dumps(evidence, indent=2))
            print(json.dumps(evidence, indent=2))
        except Exception:
            (ROOT / ".aster/qa/checkpoint-failure.json").write_text(json.dumps({"screen": screen_text(screen), "frames": len(output.frames)}, ensure_ascii=False, indent=2))
            raise
        finally:
            stop_child(child, output)


if __name__ == "__main__":
    main()
