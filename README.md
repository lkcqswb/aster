# Aster

**A terminal agent, with 弄玉 beside you.**

Aster is written in Rust. The interface is a conversation: type a request, watch the work, approve changes, and check the result. 弄玉 is rendered from her actual Live2D model *inside* your terminal. She breathes, blinks, reacts to typing, thinks while the model works, and speaks as a reply arrives.

Inspired by the conversational flow of [Claude Code](https://code.claude.com/docs/en/interactive-mode) and the [sessions and slash commands in OpenCode](https://opencode.ai/docs/tui/). See [design notes](docs/DESIGN.md) for the decisions and references.

The agent runtime is evolving toward [Pi's event-driven tools and extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md). Aster now supports precise file edits, structured work plans, questions during a task, and a companion work card backed by actual tool results. See the [delivery plan](docs/EVOLUTION.md).

## Start

Requires a stable Rust toolchain. Live2D additionally uses Chrome/Chromium and local model assets; the agent works without them.

```sh
cargo build --release
./aster
```

Run `aster` from the project you want to work on. It preserves your current directory. You can also select a project explicitly:

```sh
aster --project ~/your-project
aster --continue
aster --resume SESSION_ID
aster --demo
```

The launcher uses the compiled Rust binary. It builds on first launch if necessary. To add it to zsh, use your checkout's absolute path:

```sh
aster() { "/absolute/path/to/aster/aster" "$@"; }
```

Put that function in `~/.zshrc`, then run `source ~/.zshrc` once. No Python runtime is needed for the new app.

## Meet 弄玉

The companion pane is part of the terminal layout, not a separate window. iTerm2 uses inline PNG frames; Ghostty and Kitty use the Kitty graphics protocol. Aster detects these terminals and has an animated character-cell fallback. Select a protocol explicitly with `--graphics iterm`, `--graphics kitty`, or `--graphics halfblocks`.

- Start typing: she becomes attentive.
- Send a message: she thinks while waiting, works during tools, and speaks during streamed text.
- Pass a file check: a brief pleased reaction. Fail or encounter an error: a concerned state.
- Click the portrait or use `/look` for a glance and nod.
- `/mood happy`, `/mood heart`, `/mood angry`, `/mood neutral` change her expression.
- `/pet off` releases the renderer; `/pet on` starts it again.
- `/pet retry` restarts the renderer after a failed launch. Loading stages and the actual error appear in the companion pane; `/status` includes the failed stage.

Her work card tracks the active plan step, file or command, pending decision, and recorded checks. Click her portrait or press **F2** to inspect the plan and evidence. **F3** reviews the exact file diffs from this turn. Questions and approval prompts stay beside her in a wide terminal, so she remains visible while you decide. Use `/demo work` to try the entire choice → approval → edit → verification flow without an API call.

Reading and checking direct her gaze toward the work; a question keeps her attentive until answered. A completed plan is separate from verification: old checks from an earlier turn never make a new task appear verified, and failed checks remain visible even if a later check passes.

Speaking motion follows text activity. This version does not synthesize speech or claim audio lip sync. The companion is a fictional AI character.

By default Aster reads existing assets here:

```text
~/desktop-pet/assets/
  vendor/pixi.min.js
  vendor/live2dcubismcore.min.js
  vendor/pixi-live2d-display-cubism4.min.js
  弄玉运行档_无水印/
    弄玉.model3.json
    弄玉.moc3
    弄玉.physics3.json
    弄玉.cdi3.json
    弄玉.4096/texture_*.png
```

Use `--pet-dir /path/to/assets` or `ASTER_PET_DIR` for another location. `--chrome /path/to/chromium` or `ASTER_CHROME` selects the renderer executable. Asset paths are checked and the renderer serves only a model-file allowlist on an ephemeral loopback address.

A Rust-owned, isolated headless Chromium process runs the existing Cubism Web SDK, then sends real model frames to the Rust TUI at up to 8 fps. It uses a temporary browser profile and closes with Aster. The terminal, sessions, agent loop, tools, instruction loading and provider client are Rust; Live2D's existing Web SDK and the small drawing bridge are JavaScript. No website UI opens. The model and vendor SDK files are **local dependencies and are not distributed in this repository**.

Startup diagnostics are saved privately in `~/.local/share/aster/diagnostics/live2d.json` (or under your selected `--state-dir`). This is a record of the latest startup or error transition, not a continuously updated frame counter. It contains no conversation or provider key. Renderer traffic stays on loopback and bypasses proxy settings. Browser stderr is retained only in its temporary profile, with a bounded excerpt included when startup fails.

The renderer defaults to a 2048-pixel texture limit. The nine original 4096-pixel textures are reduced in browser memory for the terminal portrait, using 75% fewer texture pixels; source assets are never rewritten. Use `--texture-size 4096` for native textures or `--texture-size 1024` for a smaller profile. `/status` reports the actual loaded texture sizes and loading progress. Texture-pixel reduction is not a claim about total process memory.

The local renderer makes at most one automatic retry after a startup failure. Missing model/Chrome paths fail immediately; provider API requests never retry automatically.

## Conversations and commands

Type `/` for a searchable command menu. Tab completes; Enter chooses. `Ctrl+P` opens the session picker. `Esc` closes a panel or stops the current turn. `Ctrl+J` adds a line; Enter sends. Page Up/Down scroll the conversation. `Ctrl+T` expands tool details. `Ctrl+C` saves and exits.

| Command | What it does |
| --- | --- |
| `/new [title]` | Save this conversation and start another |
| `/sessions`, `/resume ID` | Find and continue a project session |
| `/rename TITLE` | Rename the current conversation |
| `/fork [title]` | Branch its model context and transcript |
| `/compact` | Archive full context; retain four recent exchanges and a local excerpt |
| `/export` | Export a readable Markdown transcript |
| `/delete` | Confirm removal of the current saved conversation |
| `/context` | Inspect model context, loaded skills and attached files |
| `/skills`, `/skill NAME request` | Find, inspect and apply a reusable skill |
| `/prompts`, `/prompt NAME args` | Browse and use reusable task prompts |
| `/reload` | Refresh and inspect project resource discovery |
| `/agents` | Inspect the AGENTS.md files loaded for this project |
| `/init` | Create a small AGENTS.md starter, without replacing an existing one |
| `/plan`, `/build` | Switch between reading/planning and work with tools |
| `/permissions ask\|allow\|deny` | Control file-write and shell approval |
| `/model live\|demo\|NAME` | Select MiniMax, the offline demo, or a model name |
| `/check FILE [JSON]` | Check existence, or compare saved JSON with an expected value |
| `/run COMMAND` | Run a local command without calling the model |
| `/output` | Watch current command output, elapsed time and final status |
| `/recover` | Ask the model to investigate the latest command failure |
| `/work` | Inspect the current task plan, files and independent evidence |
| `/review` | Review diffs from file tools in this turn |
| `/steer MESSAGE` | Redirect current work at the next tool boundary |
| `/follow MESSAGE` | Queue the next task after a normal finish |
| `/queue`, `/next`, `/drop ID` | Inspect, resume or remove waiting messages |
| `/tools` | Expand or collapse tool output |
| `/mood`, `/look`, `/pet` | Interact with the character |
| `/demo` | Run a scripted, real file-write and verification example |
| `/status`, `/help`, `/stop`, `/quit` | Inspect, learn, interrupt, leave |

**F5 /checks** opens the evidence behind 弄玉's reaction. A passing rerun of the same check resolves its earlier failure while preserving both outcomes. A different assertion cannot erase a failure. File-tool edits make previous passing checks stale; her card asks for a fresh check instead of celebrating. This tracks Aster's file edits, not arbitrary shell or external changes. `/check` runs locally and saves its actual result for the next model turn. Try `/demo evidence` and `/demo evidence stale` offline.

While work runs, **Enter steers** and **Alt+Enter queues a follow-up**. **Ctrl+G** opens a direction box, including during a question or approval; Escape returns to the existing decision. Sending a correction cancels pending decisions and skips tool calls that have not started. An already-running command can finish; use Escape to stop it. 弄玉 acknowledges the new direction and her work card follows the updated task.

The queue is saved with the conversation (up to eight messages / 32 KB). A normal finish advances it; a stop, error or restart leaves it waiting for `/next`. Forks start with an empty queue so the same pending work does not run in two conversations.

On Unix, terminal hangup and termination signals request a saved, orderly shutdown. Active commands are cancelled with their background children; the private renderer and its profile are removed. An interrupted provider request may still consume tokens. A forced process kill cannot run cleanup or save new state.

Forks share the project filesystem. They do not roll back files. Compaction is deterministic local context reduction, not an LLM-generated summary; the full earlier session is archived privately before reduction.

Sessions live in `~/.local/share/aster`, with private files, atomic writes and an exclusive store lock. Use `--state-dir` for a separate store. Session listing is scoped to the project. No API key is stored in session files. Exports omit private provider content such as thinking blocks.

## Project rules and tools

Aster loads `~/.config/aster/AGENTS.md`, followed by ancestor `AGENTS.md` files from broad to narrow scope. Use `/agents` to see exactly which files were included. When a file tool reaches a nested directory with new guidance, that guidance is returned to the model before the operation is retried. Instruction loading is bounded by file size.

Attach project context with `@path`, `@path:10-30` or `@{path with spaces}`. Skills load on demand from personal or project directories; the searchable picker stays beside 弄玉, and her work card records which skill is in use. [Context, skills and prompt templates](docs/CONTEXT.md) explains limits, locations and examples.

The tools are `read_skill`, `list_files`, `read_file`, `search`, `write_file`, `edit_file`, `shell`, `check_file`, `update_plan` and `ask_user`. File tools reject path traversal, symlinks and credential/private directories. Reads and writes are capped at 128 KB, file lists at 800 entries and searches at 100 matching lines. `read_file` returns numbered lines, with a one-based offset and up to 500 lines per call (200 by default).

`edit_file` replaces a single exact occurrence and rejects ambiguous matches. File edits are prepared before approval, displayed as a diff, and committed atomically only if the file still matches the reviewed version. Intervening user edits are preserved. `update_plan` reports progress; it cannot manufacture verification evidence. `ask_user` waits for a numbered choice or a typed answer, and returns that answer to the model before dependent work continues.

By default each write or shell command is shown for approval. `y` allows that action once; `n` or Escape declines it. Plan mode forbids writes and shell commands regardless of the approval setting. **Approved shell commands run as your user and are not a filesystem sandbox.** Credential environment variables are removed from their environment. Commands default to 30 seconds; the model can request 1–120 seconds, bounded by the remaining active turn time. The approval shows the command and requested limit. Output streams into **F4 /output** beside 弄玉, and clicking her during a command opens that view. The final record distinguishes exit status, timeout and your stop action. Capture retains the first and last 16 KB of each stream, while the live panel shows its latest 4 KB. Process-group cancellation stops background children too.

`/run COMMAND` uses the same permission and plan-mode rules with no model request. After a failure, `/recover` starts a new model turn to inspect the evidence, make a focused repair and check it. It does not automatically repeat the command or approve another action. Try `/demo command`, `/demo command timeout` and `/demo command stop` for local examples.

A completed reply means the model finished speaking. A passed check means a specific saved-file assertion was evaluated successfully. Neither alone proves the entire project is correct. Read the actual check or test output.

## MiniMax

Copy `.env.example` to `.env` in the Aster checkout, replace the placeholder and restrict its permissions:

```sh
chmod 600 .env
```

Aster reads only the named MiniMax settings from this file; environment variables take precedence. The default model is `MiniMax-M2.7`, via MiniMax's Anthropic-compatible endpoint. Responses stream into the conversation, including streamed tool arguments. Full assistant blocks are retained privately for provider-compatible continuation. Provider redirects are rejected.

Each user turn permits at most 12 model requests, 24 tool calls, 180 active seconds, 2,048 output tokens per request and 12,000 output tokens overall. Question and approval waits pause the active timer, with a 15-minute maximum per decision. Unexecuted or declined checks are not recorded as failed tests. These are work limits, not a currency cap; input tokens also incur usage. Errors and truncated streams stop without automatic retries. A submitted request may finish and consume tokens after local cancellation.

For command-line use without the TUI:

```sh
aster --project /path/to/project --prompt 'Explain this project'
aster --demo --permissions allow --prompt 'demo task'
```

Non-interactive mode declines actions that need confirmation unless `--permissions allow` was explicitly selected. A headless `/run` exits nonzero when its command fails, times out, is stopped or is denied.

## Development and verification

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

The optional real-PTY test driver uses the existing development-only Python test dependencies:

```sh
.venv/bin/python scripts/test_rust_e2e.py
.venv/bin/python scripts/test_rust_e2e.py --live2d
.venv/bin/python scripts/test_live2d_startup.py
.venv/bin/python scripts/test_companion_work.py
```

To validate the real model independently and save distinct animation frames:

```sh
aster --live2d-probe .aster/qa/model
aster --screenshot .aster/qa/workbench.svg
```

The main test driver uses demo mode and makes no API calls. `.venv/bin/python scripts/test_rust_live.py` explicitly runs one bounded real MiniMax task through the TUI, approves its expected file write, and checks the result. Local QA assets are ignored by Git. [Validation results](VALIDATION.md) distinguish automated tests, actual renderer checks and live-model results.

`.venv/bin/python scripts/test_agent_edit_live.py --run-live` explicitly runs a real coding task: inspect a broken Python function and its tests, make a precise approved edit, run the approved test command, then independently rerun the unchanged tests. It makes one bounded user turn and never retries a failed provider request.

The prior Python prototype is retained as historical source; its instructions are in [legacy documentation](docs/legacy-prototype.md). It is not used by the `aster` command.
