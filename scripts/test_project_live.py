"""One explicit MiniMax search/read/edit/check task. No retries or broad approvals."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time

import pexpect
import pyte
from terminal_helpers import screen_text, stop_child

ROOT = Path(__file__).resolve().parents[1]
RECORD = "target_record=486 mode=bad proof=jadesearch-739"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-live", action="store_true")
    if not parser.parse_args().run_live:
        parser.error("--run-live is required; this consumes provider tokens")
    qa = ROOT / ".aster/qa"
    state = qa / "project-live-sessions"
    with tempfile.TemporaryDirectory(prefix="aster-project-live-") as temporary:
        project = Path(temporary).resolve()
        (project / "src/archive").mkdir(parents=True)
        filename = project / "src/archive/corpus.txt"
        original = "".join(f"record_{i:05d}=keep-this-existing-data-intact\n" for i in range(8000))
        original += RECORD + "\nend_of_catalog=true\n"
        filename.write_text(original)
        (project / ".gitignore").write_text("src/ignored.txt\n")
        (project / "src/ignored.txt").write_text("TARGET_RECORD=0 mode=wrong\n")
        (project / "AGENTS.md").write_text("# Fixture\nUse only read-only navigation, update_plan, edit_file and check_file. No shell commands or write_file. Change only src/archive/corpus.txt with a precise edit, preserving all unrelated bytes.\n")
        (project / "src/archive/AGENTS.md").write_text("# Catalog\nRead a small numbered range around the record before editing. Verify the full changed record with check_file contains.\n")
        child = pexpect.spawn(str(ROOT / "aster"), ["--project", str(project), "--state-dir", str(state), "--no-live2d"],
                              env=dict(os.environ, TERM="xterm-256color"), encoding="utf-8", codec_errors="replace", dimensions=(42, 132))
        screen = pyte.Screen(132, 42)
        stream = pyte.Stream(screen)

        def pump():
            try:
                stream.feed(child.read_nonblocking(65536, timeout=.1))
            except pexpect.TIMEOUT:
                pass
            return screen_text(screen)

        def session():
            for path in state.glob("*.json"):
                value = json.loads(path.read_text())
                if value["project"] == str(project):
                    return value
            return None

        approved = False
        result = None
        try:
            deadline = time.monotonic() + 10
            while "aster" not in pump():
                assert time.monotonic() < deadline
            prompt = "Share a short plan, then find TARGET_RECORD in src using search with regex=true, case_sensitive=false, glob=src/**/*.txt. Read at most 5 nearby lines using read_file's offset and limit. The source is larger than 128 KB. Change only mode=bad to mode=good in that record using edit_file. Preserve every other byte. Verify the full updated record using check_file with kind=contains. Do not use shell or write_file. Report the actual result briefly."
            child.send("\x1b[200~" + prompt + "\x1b[201~\r")
            deadline = time.monotonic() + 190
            while time.monotonic() < deadline:
                text = pump()
                assert "Allow write_file?" not in text and "Allow shell?" not in text, "Unexpected mutation requested"
                if "Allow edit_file?" in text and "mode=bad" in text and "mode=good" in text and "corpus.txt" in text and not approved:
                    assert filename.read_text() == original
                    child.send("y")
                    approved = True
                candidate = session()
                if candidate and candidate["status"] in ("done", "error", "stopped"):
                    result = candidate
                    break
            assert result and result["status"] == "done", "Task did not finish successfully"
            assert approved
            expected = original.replace(RECORD, RECORD.replace("mode=bad", "mode=good"))
            assert filename.read_text() == expected
            calls = [block for message in result["messages"] if isinstance(message.get("content"), list)
                     for block in message["content"] if block.get("type") == "tool_use"]
            assert any(call["name"] == "search" and call["input"].get("regex") is True
                       and call["input"].get("case_sensitive") is False for call in calls)
            assert any(call["name"] == "read_file" and call["input"].get("offset", 1) > 7990
                       and call["input"].get("limit", 200) <= 5 for call in calls)
            assert any(e["passed"] for e in result["work"]["evidence"])
            assert result["work"]["changed"] == ["src/archive/corpus.txt"]
            assert "corpus.txt:8001" in result["work"]["discovery"]
            assert all(call["name"] not in ("shell", "write_file") for call in calls)
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
            evidence = {"session": result["id"], "model": result["model"], "model_requests": result["work"]["model_requests"],
                        "tools": result["tools"], "input_tokens": result["input_tokens"], "output_tokens": result["output_tokens"],
                        "source_bytes": len(original.encode()), "target_line": 8001, "scoped_regex_search": True,
                        "small_numbered_read": True, "precise_edit_approved": True, "unrelated_bytes_unchanged": True,
                        "independent_full_file_sha256": hashlib.sha256(filename.read_bytes()).hexdigest(),
                        "exact_expected_sha256": hashlib.sha256(expected.encode()).hexdigest(), "exit": 0}
            (qa / "project-live.json").write_text(json.dumps(evidence, indent=2))
            print(json.dumps(evidence, indent=2))
        except Exception:
            result = session()
            (qa / "project-live-failure.json").write_text(json.dumps({"session": result["id"] if result else None,
                "input_tokens": result.get("input_tokens", 0) if result else 0,
                "output_tokens": result.get("output_tokens", 0) if result else 0,
                "approved": approved, "screen": screen_text(screen)}, ensure_ascii=False, indent=2))
            raise
        finally:
            stop_child(child, stream)


if __name__ == "__main__":
    main()
