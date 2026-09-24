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

The companion pane is part of the terminal layout, not a separate window. iTerm2 receives inline JPEG frames; Ghostty and Kitty receive PNG frames through the Kitty graphics protocol, replaced in place under one image id. Aster detects these terminals and has an animated character-cell fallback that draws each cell as a two-colour quadrant block (four pixels per cell). Select a protocol explicitly with `--graphics iterm`, `--graphics kitty`, or `--graphics halfblocks`.

- Start typing: she becomes attentive.
- Send a message: she thinks while waiting, works during tools, and speaks during streamed text.
- Pass a file check: a brief pleased reaction. Fail or encounter an error: a concerned state.
- Click the portrait or use `/look` for a glance and nod.
- `/mood happy`, `/mood heart`, `/mood angry`, `/mood neutral` change her expression.
- `/pet off` releases the renderer; `/pet on` starts it again.
- `/pet retry` restarts the renderer after a failed launch. Loading stages and the actual error appear in the companion pane; `/status` includes the failed stage.

Replies render headings, emphasis, lists, code blocks and tables with restrained terminal styling. Code keeps its indentation; approvals and reviews color additions and removals. [Reading and formatting](docs/FORMATTING.md) describes the display behavior.

Her work card tracks the active plan step, file or command, pending decision, and recorded checks. Click her portrait or press **F1** for the searchable action menu; **F2** goes directly to the plan and evidence. **F3** reviews the exact file diffs from this turn. Questions and approval prompts stay beside her in a wide terminal, so she remains visible while you decide. Use `/demo work` to try the entire choice → approval → edit → verification flow without an API call. During a pending question or approval, F2–F6 open the plan, diffs, output, checks and files. Escape returns to the same decision with its typed answer or scroll position preserved. An inspection cannot approve an action; edits still reject any file changed since its diff was prepared.

Reading and checking direct her gaze toward the work; a question keeps her attentive until answered. A completed plan is separate from verification: old checks from an earlier turn never make a new task appear verified, and failed checks remain visible even if a later check passes.

Her motion is procedural and eased: every state change moves her toward a new pose instead of snapping, with randomized blinks, small eye saccades, a head that follows her gaze, and slow breathing and sway. Speaking motion follows the rate of streamed reply text: syllable-like mouth pulses while text arrives, closing shortly after it stops. This version does not synthesize speech or claim audio lip sync. If the model declares expressions or motions, a matching mood expression, a tap motion and occasional idle motions blend with that procedural layer. The companion is a fictional AI character.

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

Use `--pet-dir /path/to/assets` or `ASTER_PET_DIR` for another location. `--chrome /path/to/chromium` or `ASTER_CHROME` selects the renderer executable. Asset paths are checked and the renderer serves only a model-file allowlist on an ephemeral loopback address. Pose, user data, expression and motion JSON files referenced by the model3.json are included in that allowlist when present.

A Rust-owned, isolated headless Chromium process runs the existing Cubism Web SDK, then sends real model frames to the Rust TUI at about 15 fps, rendered at the portrait's pixel size and spaced further apart when the renderer needs more time per frame. Taller portrait panes show more of her, up to about half her body; wider panes keep a bust. `/status` reports the measured frame rate. It uses a temporary browser profile and closes with Aster. The terminal, sessions, agent loop, tools, instruction loading and provider client are Rust; Live2D's existing Web SDK and the small drawing bridge are JavaScript. No website UI opens. The model and vendor SDK files are **local dependencies and are not distributed in this repository**.

Startup diagnostics are saved privately in `~/.local/share/aster/diagnostics/live2d.json` (or under your selected `--state-dir`). This is a record of the latest startup or error transition, not a continuously updated frame counter. It contains no conversation or provider key. Renderer traffic stays on loopback and bypasses proxy settings. Browser stderr is retained only in its temporary profile, with a bounded excerpt included when startup fails.

The renderer defaults to a 2048-pixel texture limit. The nine original 4096-pixel textures are reduced in browser memory for the terminal portrait, using 75% fewer texture pixels; source assets are never rewritten. Use `--texture-size 4096` for native textures or `--texture-size 1024` for a smaller profile. `/status` reports the actual loaded texture sizes and loading progress. Texture-pixel reduction is not a claim about total process memory.

The local renderer makes at most one automatic retry after a startup failure. Missing model/Chrome paths fail immediately; provider API requests never retry automatically.

## Conversations and commands

Type `/` for a searchable command menu; ↑↓ choose, Tab completes, Enter chooses and Esc closes it. `Ctrl+P` opens your conversations in this project. Enter sends, `Ctrl+J`, `Shift+Enter` or a trailing `\` adds a line, and `Ctrl+C` saves and exits (with a draft, the first press clears it).

**Writing.** ↑/↓ move between the lines of a longer draft. From the first or last line they walk through your earlier requests in this project, and ↓ past the newest brings back the unsent draft. `Alt/Ctrl+←→` or `Alt+B/F` (Option-arrows on macOS) move by word, and Home/End go to the start/end of the line; press them again for the whole draft. `Ctrl+W` or `Alt+Backspace` deletes a word, and `Ctrl+U`/`Ctrl+K` delete to the start/end of the line. One Esc warns and a second Esc clears the draft; ↑ brings it back.

**Reading.** PageUp/PageDown move by a page, `Shift+↑↓` or the mouse wheel move three lines, and `Ctrl+Home` goes to the first message. Scrolling stops at both ends. While you read earlier messages, new replies do not move the view; a marker shows how many newer lines wait below. `Ctrl+End` or Esc returns to the latest. `Ctrl+O` (or `Ctrl+T`) expands tool details.

**Conversations.** In `Ctrl+P` / `/sessions`, typing filters by title, ID or your first requests. Each row shows when it was last active, its message count and model, with `● current` on the open one. Enter opens the highlighted conversation, which starts on the most recent *other* one. `Ctrl+N` starts a new conversation and `Ctrl+D` then `y` deletes the highlighted one (never the open one; use `/delete` for that). Opening a conversation saves the current one first. Every list panel uses ↑↓, PgUp/PgDn and Home/End.

The footer shows short hints, the context meter and the session's input/output tokens. Notices fade after a few seconds.

| Command | What it does |
| --- | --- |
| `/new [title]` | Save this conversation and start another |
| `/sessions`, `/resume ID` | Find and continue a project session |
| `/rename TITLE` | Rename the current conversation |
| `/fork [title]` | Branch its model context and transcript |
| `/compact [note]` | Archive full context; 弄玉 summarizes older exchanges, recent ones stay intact |
| `/compact local [note]`, `/compact auto on\|off` | Local excerpts with no request; turn auto-compact on or off |
| `/limits` | Show the turn limits and context window in effect |
| `/checkpoint` | Inspect retained context, byte counts and the restore ID |
| `/restore ID` | Restore archived provider context as a new conversation; files stay shared |
| `/export` | Export a readable Markdown transcript |
| `/delete` | Confirm removal of the current saved conversation |
| `/history [query]` | Search the visible conversation; F7 opens history |
| `/context` | Inspect model context, loaded skills and attached files |
| `/skills`, `/skill NAME request` | Find, inspect and apply a reusable skill |
| `/prompts`, `/prompt NAME args` | Browse and use reusable task prompts |
| `/reload` | Refresh and inspect project resource discovery |
| `/agents` | Inspect the AGENTS.md files loaded for this project |
| `/together` | Open local companion controls; F1 or click her portrait |
| `/files [name or glob]`, `/find TEXT` | Browse source and search beside 弄玉; F6 opens files |
| `/init` | Create a small AGENTS.md starter, without replacing an existing one |
| `/plan`, `/build` | Switch between reading/planning and work with tools |
| `/permissions ask\|allow\|deny` | Control file-write and shell approval |
| `/model live\|demo\|NAME` | Select MiniMax, the offline demo, or a model name |
| `/check FILE [JSON]` | Check existence, or compare saved JSON with an expected value |
| `/run COMMAND` | Run a local command without calling the model |
| `/tasks [filter]`, `/task NAME` | Inspect or run project tasks; F8 opens the picker |
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

Forks share the project filesystem. They do not roll back files.

**Auto-compact.** The footer's `ctx` meter estimates how full the model's context window is. Before a request would cross 80% of it, Aster archives the full conversation privately and replaces older exchanges with a summary that 弄玉 writes in one bounded, tool-free request. Recent exchanges stay intact. If the summary request fails, Aster uses local excerpts instead and says so; it never retries automatically. `/checkpoint` shows the summary and `/restore ID` brings back the archived context as a new conversation. [Context and compaction](docs/CONTEXT.md#context-size-auto-compact-and-checkpoints) has the details.

Sessions live in `~/.local/share/aster`, with private files, atomic writes and an exclusive store lock. Use `--state-dir` for a separate store. Session listing is scoped to the project. No API key is stored in session files. Exports omit private provider content such as thinking blocks.

## Project rules and tools

Aster loads `~/.config/aster/AGENTS.md`, followed by ancestor `AGENTS.md` files from broad to narrow scope. Use `/agents` to see exactly which files were included. When a file tool reaches a nested directory with new guidance, that guidance is returned to the model before the operation is retried. Instruction loading is bounded by file size.

Attach project context with `@path`, `@path:10-30` or `@{path with spaces}`. Skills load on demand from personal or project directories; the searchable picker stays beside 弄玉, and her work card records which skill is in use. [Context, skills and prompt templates](docs/CONTEXT.md) explains limits, locations and examples.

弄玉 has sixteen tools:

| Kind | Tools | Approval |
| --- | --- | --- |
| Read | `list_files`, `search`, `read_file`, `read_files` (up to 8 files per call), `outline` (definitions with line numbers), `read_skill` | none |
| Change files | `write_file`, `edit_file`, `multi_edit` (several exact edits to one file, one diff), `move_file`, `delete_file` (one regular file, never a directory) | per action in ask mode; refused in plan mode |
| Run | `shell` | per action in ask mode; refused in plan mode |
| Network | `web_fetch` (one HTTP(S) GET, readable text, public addresses only) | per action in ask mode, also in plan mode; refused in deny mode |
| Work | `check_file`, `update_plan`, `ask_user` | none |

Every file change is prepared first, shown as the exact diff or rename you approve, and refused at commit if the file changed in between. `web_fetch` returns page text marked as untrusted data. It refuses private, loopback and link-local addresses at every redirect and sends no cookies or credentials. File tools reject path traversal, symlinks and credential/private directories. Navigation respects ignore rules and supports directory/glob filters, optional regular expressions, case control and explicit pagination. Reads and focused edits support UTF-8 files up to 2 MB; whole-file writes remain capped at 128 KB. Numbered read pages are bounded to 16 KB and continue across long lines without losing characters. [Project navigation and editing](docs/PROJECT_TOOLS.md) describes the controls and limits.

`edit_file` replaces a single exact occurrence and rejects ambiguous matches. File edits are prepared before approval, displayed as a diff, and committed atomically only if the file still matches the reviewed version. Intervening user edits are preserved. `update_plan` reports progress; it cannot manufacture verification evidence. `ask_user` waits for a numbered choice or a typed answer, and returns that answer to the model before dependent work continues.

By default each write or shell command is shown for approval. `y` allows that action once; `n` or Escape declines it. Plan mode forbids writes and shell commands regardless of the approval setting. **Approved shell commands run as your user and are not a filesystem sandbox.** Credential environment variables are removed from their environment. Commands default to 30 seconds; the model can request 1–600 seconds, bounded by the remaining active turn time. The approval shows the command and requested limit. Output streams into **F4 /output** beside 弄玉, and clicking her during a command opens that view. The final record distinguishes exit status, timeout and your stop action. Capture retains the first and last 16 KB of each stream, while the live panel shows its latest 4 KB. Process-group cancellation stops background children too.

**F8 /tasks** discovers project tests, scripts and builds, or reads your explicit `.aster/tasks.json`. Inspect a command beside 弄玉, then run it with the usual permissions and no model request. [Project tasks](docs/TASKS.md) describes discovery, configuration and automation outcomes.

`/run COMMAND` uses the same permission and plan-mode rules with no model request. After a failure, `/recover` starts a new model turn to inspect the evidence, make a focused repair and check it. It does not automatically repeat the command or approve another action. Try `/demo command`, `/demo command timeout` and `/demo command stop` for local examples.

A completed reply means the model finished speaking. A passed check means a specific saved-file assertion was evaluated successfully. Neither alone proves the entire project is correct. Read the actual check or test output.

## Models and API keys

Type `/models` (or choose **Models and API keys** from F1) to see every model you can use, add a provider or key, and switch models:

- **Enter** uses the highlighted model for this conversation. New conversations start with it too.
- **a** adds a provider. Presets fill in **Anthropic** (`https://api.anthropic.com`, `x-api-key`, `claude-opus-5`, `claude-sonnet-5`, `claude-haiku-4-5`) and **MiniMax**. **Custom** covers any service with an Anthropic-compatible Messages endpoint, including a local proxy on `http://localhost`.
- The **API key** field hides what you type or paste. **k** replaces a key; leaving the field empty while editing keeps the saved one.
- **m** edits a provider's model names, comma-separated. `name=tokens` records a context window, which the context meter and auto-compact use.
- **t** tests the highlighted model with one tiny request (a few tokens), explicitly and without retrying. It reports the HTTP status, and on failure whether to check the key, base URL or model name.
- **d** removes a provider and its key after you confirm.

Keys are saved only in `providers.json` beside your private sessions (`~/.local/share/aster`, owner-only, or your `--state-dir`). A key is sent only to its own provider's base URL, as `Authorization: Bearer` or `x-api-key`, whichever you chose. It never appears in sessions, transcripts, exports, notices, diagnostics or debug output; the panel shows only its last four characters. Base URLs must be https; plain http is allowed only for localhost. File tools cannot read the store, but approved shell commands run as your user and could, as with `.env`.

**MiniMax from `.env`** still works without the panel. Copy `.env.example` to `.env` in the Aster checkout, replace the placeholder and restrict its permissions:

```sh
chmod 600 .env
```

Aster reads only the named MiniMax settings from this file; environment variables take precedence, and their base URL must be an official MiniMax endpoint. The default model is `MiniMax-M2.7`. Responses stream into the conversation, including streamed tool arguments. Full assistant blocks are retained privately for provider-compatible continuation. Provider redirects are rejected. `/model NAME` switches the model name within the current provider; `/model demo` and `/model live` switch the offline demo on and off.

By default each user turn permits at most 40 model requests, 120 tool calls, 900 active seconds (15 minutes), 8,192 output tokens per request and 64,000 output tokens overall. Shell commands may run up to 600 seconds each, within the turn's remaining time. Change these when starting Aster, for example `aster --max-requests 60 --turn-seconds 1800`, or with `ASTER_MAX_REQUESTS`, `ASTER_MAX_TOOLS`, `ASTER_TURN_SECONDS`, `ASTER_MAX_OUTPUT_TOKENS`, `ASTER_TURN_OUTPUT_TOKENS`, `ASTER_CONTEXT_WINDOW` and `ASTER_AUTO_COMPACT`. `/limits` shows the values in effect. Reaching a limit stops the turn with a message; send a new message to continue. Question and approval waits pause the active timer, with a 15-minute maximum per decision. Unexecuted or declined checks are not recorded as failed tests. These are work limits, not a currency cap; input tokens also incur usage. Errors and truncated streams stop without automatic retries. A submitted request may finish and consume tokens after local cancellation.

For command-line use without the TUI:

```sh
aster --project /path/to/project --prompt 'Explain this project'
aster --demo --permissions allow --prompt 'demo task'
```

Non-interactive mode declines actions that need confirmation unless `--permissions allow` was explicitly selected. A headless `/run` or `/task` exits nonzero when its command fails, times out, is stopped or is denied. Other headless turns also fail when recorded checks are currently failing or stale; a reply without checks can finish successfully without claiming verification.

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
