> Historical Python prototype documentation. The active `aster` launcher now runs Rust. To open the old TUI explicitly, use `.venv/bin/python -m relay tui` from the repository root.

# Aster

A small local agent harness: **task → model → tools → independent checks**.

Includes a terminal workbench, a web dashboard, a command-line runner, MiniMax's Anthropic-compatible API, explicit tool registration, bounded runs, saved traces, and branching from a checkpoint. Python 3.10+ on macOS or Linux. The engine and web dashboard use the standard library; the optional TUI uses Textual and Pillow.

## Terminal workbench

From the project directory, install the terminal dependencies and launch:

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements-tui.txt
./aster
```

For live MiniMax tasks, copy `.env.example` to `.env`, replace the placeholder with your API key, and run `chmod 600 .env`. Choose the scripted demo to try the workbench without a key. The private `.env` file is excluded from Git.

To launch with `aster` from any directory, add this function to `~/.zshrc`, using your checkout's absolute path, then run `source ~/.zshrc`:

```sh
aster() { "/absolute/path/to/aster/aster" "$@"; }
```

The internal Python package and saved-data directory remain `relay` and `.relay` for compatibility with existing runs.

The workbench uses a charcoal-and-sage palette, a live execution timeline, a file browser, independent checks, checkpoint branching, and local JSON export. It adapts to an 80×24 terminal; the history sidebar and companion appear when space permits. Terminal `NO_COLOR` preferences are respected.

| Key | Action |
| --- | --- |
| `n` | New task: choose MiniMax or the scripted demo, then set limits |
| `1`–`4` | Timeline, checks, files, original task |
| `h` | Focus/toggle run history; arrows and Enter select a run |
| `b` | Branch from a saved checkpoint with an optional new instruction |
| `s` | Stop the active run |
| `e` | Export the current trace and files to `.relay/exports/` |
| `c` | Toggle the portrait companion |
| `r` | Refresh history |
| `q` | Quit; local active work prompts to stop, attached server work continues |
| `Esc` | Close a form |
| `Tab` / `Shift+Tab` | Move between controls |

The TUI opens the same saved run directory as the dashboard. If that directory is already served on port 8787, it attaches after verifying the workspace identity. You can also connect explicitly:

```sh
./aster --connect http://127.0.0.1:8787
./aster --provider demo --no-companion
.venv/bin/python -m relay --data-dir /tmp/relay-tui-test tui
```

**Companion:** Aster reads the portrait bundled with the user's existing Live2D model at `~/desktop-pet/assets/弄玉运行档_无水印/3icon.png` and renders it with colored terminal half blocks. It does not modify the desktop pet, redistribute its assets, or run Live2D animation in the terminal. Use `--companion /path/to/image.png` for another local portrait; a text mascot appears if the image is unavailable.

**Custom tasks:** choose “Custom task / manifest” in New run. Enter an instruction, or supply a task JSON file such as `examples/custom-task.json`. A manifest defines starting files and success checks. A prompt without checks gets an **unverified** result rather than a passing grade.

### Terminal tests

```sh
.venv/bin/python -m pip install -r requirements-test.txt
.venv/bin/python -m unittest discover -s tests -v
.venv/bin/python scripts/test_terminal_e2e.py
```

The suite uses [Textual's documented test driver](https://textual.textualize.io/guide/testing/) plus a real pseudo-terminal test with actual keyboard input, screen parsing, file inspection, export, resize, and clean exit. These commands make no model API calls. The **explicit** `--live` option runs one bounded MiniMax task through the TUI and requires the dashboard to be running:

```sh
.venv/bin/python scripts/test_terminal_e2e.py --live
```

Test evidence is saved under `.relay/qa/`. Run exports contain task files and public events; private provider conversation checkpoints are excluded.

## Web dashboard

From this project directory:

```sh
python3 -m relay serve
```

Open http://127.0.0.1:8787. Choose **New run**, a sample task, and either MiniMax or the **scripted demo**. The demo exercises real file operations and checks but does not call a model or interpret arbitrary instructions.

For MiniMax, copy `.env.example` to `.env`, supply `ANTHROPIC_AUTH_TOKEN`, and restrict the file to your user (`chmod 600 .env`). Environment variables override values in `.env`. The key never enters task workspaces, model messages, public traces, or the browser. No existing Claude/Codex settings are changed. Restart the server after changing configuration.

The configured endpoint is `https://api.minimaxi.com/anthropic`; the adapter posts to `/v1/messages`. The default model is `MiniMax-M2.7` and can be changed per live run. MiniMax's [official Anthropic compatibility documentation](https://platform.minimax.cn/docs/api-reference/text-anthropic-api) describes the tools/content-block protocol. Complete assistant content is passed back between turns, including opaque thinking blocks. The dashboard and public export show tool actions, results, and assistant text; private checkpoints also retain the full provider conversation.

## Run from the command line

```sh
python3 -m relay run --provider demo --task revenue-audit
python3 -m relay run --provider demo --task failure-lab
python3 -m relay run --provider minimax --task config-repair
python3 -m relay run --provider minimax --task-file examples/custom-task.json
python3 -m unittest discover -s tests -v
```

Aster locks its data directory to prevent two processes from interfering with the same runs. To run a CLI experiment while the dashboard is open, choose a separate directory:

```sh
python3 -m relay --data-dir /tmp/relay-experiment run --provider demo
```

## What you can inspect

- **Timeline:** real model requests, tool arguments/results, checkpoints, usage, and verification outcomes.
- **Files:** actual UTF-8 files created or changed inside each run's workspace.
- **Checks:** existence, text containment, exact text equality, or exact JSON equality. They are defined before the run and cannot be edited by tools. “Passed” only establishes those checks. No checks produces **unverified**, never “passed.”
- **Replay / branch:** restores the saved file snapshot and complete conversation at a turn boundary, then starts a new run with fresh limits. A MiniMax branch can add an instruction or change model; it makes new API calls and is not guaranteed to reproduce the same result. Checkpoint 0 restores that run's starting point. Parent artifacts and check definitions are preserved.
- **Export trace:** downloads a JSON record with task, events, usage, checks, and current files. Checkpoint conversations and credentials are not included.

## Limits and trust boundary

The default caps are 12 model turns, 24 tool calls, 180 seconds, 2,048 output tokens per call, and 12,000 output tokens per run. The next request's output allowance is reduced to the remaining budget. Provider usage is recorded, including separately reported cache tokens. These controls are **not a currency spending cap**; input tokens also incur usage and token-plan accounting can differ.

Cancellation/deadline checks stop new tool actions. An already submitted API request may finish and consume tokens after local cancellation; Aster discards its result. HTTP errors, invalid responses, and truncated output end the run without automatic retries. Failed tool calls can be corrected by the agent on a later turn within the same limits.

Tools: `list_files`, `read_file`, `write_file`, and a restricted arithmetic `calculate`. No shell, Python execution, arbitrary network access, or access to your existing project directories is offered to the model. File operations reject parent traversal and paths that resolve outside the run workspace. Each UTF-8 file is limited to 64 KB; the workspace is limited to 100 files / 1 MB.

The HTTP server binds to loopback, validates Host/Origin, rejects cross-origin writes, and serves an explicit static-file allowlist. It is a single-user local prototype, not a multi-tenant service or OS sandbox against other processes running as your user. Do not expose it to the internet. Task files and tool results are sent to MiniMax when you choose the live provider.

Local data lives in `.relay/runs/<id>/`:

```text
run.json          Run status, limits, usage, and checks
task.json         Frozen task and verification definitions
events.jsonl      Append-only public event trace
workspace/       Agent-readable and writable files
checkpoints/     File snapshots and private conversation history
```

Interrupted runs are marked on restart and never resumed automatically. Use an explicit replay to continue. Local data and `.env` are excluded by `.gitignore`.

## Extend

- `relay/providers.py`: implement `complete(messages, tools, max_tokens, timeout, task)` with the normalized Anthropic-style content protocol and a `model` name.
- `relay/tools.py`: register a schema and an executor. Keep deterministic verification separate from tool execution.
- `relay/tasks.py`: add sample manifests or load your own via the UI/CLI.
- `relay/engine.py`: model loop, budgets, checkpoints, cancellation, and trace persistence.

The default tests use mock transports and temporary workspaces. Live model calls are opt-in via the explicitly named terminal test above.
