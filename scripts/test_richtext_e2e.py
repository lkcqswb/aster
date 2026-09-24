"""Actual terminal markdown styles and reviewed diff colors beside animated Live2D."""
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
MARKDOWN = '''# Ready to review

**Plan** with `src/main.rs` and *one focused change*.

1. Read the file
2. Check the actual result

```rust
fn main() {
    println!("你好");
}
```

| Check | Result |
|:---|:---|
| Exact JSON | Passed |
| Source file | Unchanged |

[Notes](https://example.com/notes)
'''


def main():
    with tempfile.TemporaryDirectory(prefix="aster-richtext-") as temporary:
        project = Path(temporary).resolve() / "project"
        project.mkdir()
        state = Path(temporary) / "sessions"
        original = seed(project, state)
        original["messages"] = []
        original["work"] = {}
        original["entries"] = [{"role": "you", "text": "Show the plan and code clearly."}, {"role": "nongyu", "text": MARKDOWN}]
        (state / (original["id"] + ".json")).write_text(json.dumps(original))
        preview = ROOT / ".aster/qa/richtext-preview-state"
        preview.mkdir(exist_ok=True)
        preview_session = dict(original, project=str(ROOT))
        (preview / (original["id"] + ".json")).write_text(json.dumps(preview_session))
        environment = dict(os.environ, TERM="xterm-256color", TERM_PROGRAM="iTerm.app")
        environment.pop("NO_COLOR", None)  # This fixture explicitly verifies colored output.
        child = pexpect.spawn(str(ROOT / "aster"), ["--project", str(project), "--state-dir", str(state), "--resume", original["id"]],
                              env=environment,
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

        def color_at(needle):
            row, text = next((i, t) for i, t in enumerate(screen_text(screen).splitlines()) if needle in t)
            return screen.buffer[row][text.index(needle)]

        def saved():
            return json.loads((state / (original["id"] + ".json")).read_text())

        try:
            wait(lambda _: len(output.frames) >= 3, 135)
            wait(lambda text: "Ready to review" in text and "Source file" in text and "https://example.com/notes" in text)
            assert "# Ready to review" not in screen_text(screen)
            assert "**Plan**" not in screen_text(screen)
            assert color_at("Ready to review").fg == "a4c4a8", repr(color_at("Ready to review"))
            assert color_at("Plan").bold
            assert color_at("src/main.rs").fg == "d9b683"
            assert color_at("one focused change").italics
            assert "    println!" in screen_text(screen)
            child.send("/demo work\r")
            wait(lambda text: "弄玉 · a question for you" in text and "1–5 choose" in text)
            child.send("2")
            wait(lambda text: "Allow write_file?" in text and '"ready":false' in text)
            child.send("y")
            wait(lambda text: "Allow edit_file?" in text and '+{"greeting"' in text and '-{"greeting"' in text)
            assert color_at('+{"greeting"').fg == "a4c4a8"
            assert color_at('-{"greeting"').fg == "d59091"
            before = len(output.frames)
            wait(lambda _: len(output.frames) >= before + 2)
            child.send("y")
            wait(lambda _: saved()["status"] == "done")
            assert json.loads((project / "companion-demo.json").read_text()) == {"greeting": "你好", "ready": True}
            assert saved()["work"]["evidence"][-1]["passed"]
            child.send("\x1bOR")
            wait(lambda text: "Review changes" in text and '+{"greeting"' in text)
            assert color_at('+{"greeting"').fg == "a4c4a8"
            child.setwinsize(24, 80)
            screen.resize(24, 80)
            screen.reset()
            wait(lambda text: "Review changes" in text)
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
            evidence = {"markdown_heading_bold_italic_and_inline_code": True, "code_indentation": True,
                        "table_and_link_target": True, "approval_diff_colors": True, "review_diff_colors": True,
                        "real_file_and_check": True, "distinct_frames": len(output.frames), "resize": True, "api_calls": 0, "exit": 0}
            (ROOT / ".aster/qa/richtext-e2e.json").write_text(json.dumps(evidence, indent=2))
            print(json.dumps(evidence, indent=2))
        except Exception:
            (ROOT / ".aster/qa/richtext-failure.json").write_text(json.dumps({"screen": screen_text(screen), "frames": len(output.frames)}, ensure_ascii=False, indent=2))
            raise
        finally:
            stop_child(child, output)


if __name__ == "__main__":
    main()
