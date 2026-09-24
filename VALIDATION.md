# Companion workbench 0.3 — 2026-09-24

- **27 Rust tests passed**, including precise edits, ambiguous/stale edit rejection, paginated reads, a concurrent user edit during approval, question answers and paste handling, plan/evidence separation, prose/code wrapping and existing tool boundaries. Formatting, Clippy with warnings denied, and the release build passed.
- The companion PTY workflow chose a Chinese greeting, approved creation and a precise edit, verified exact saved JSON, reviewed the three-step plan and both diffs, resized the terminal, and exited cleanly. The final run captured **36 distinct native image frames**, including while questions and approvals were visible beside the character. Evidence: `.aster/qa/companion-work-e2e.json`.
- The broader terminal workflow and the startup recovery test passed. A simulated browser failure exhausts the single automatic local startup retry, then `/pet retry` recovers and produces distinct frames. This does not retry provider API calls.
- A real **MiniMax-M2.7** coding task, session `1b4247346c78`, read a broken Python function and its tests, shared a plan, made an approved `edit_file` change, and ran the approved test command. All **three unchanged tests passed**, and a separate local rerun also passed. Usage: **7 model turns, 9 tools, 4,688 input tokens, 696 output tokens**. One live task was invoked; no provider retry was made. Evidence: `.aster/qa/agent-edit-live.json`.
- The actual draw function and model frame were rendered for visual QA in `.aster/qa/aster-work.svg` and `.aster/qa/aster-work.png`. This caught and corrected mid-word text wrapping and an internal tool name appearing as the companion's focus. The work card now shows task evidence and the relevant file.
- Some development renderer launches timed out while loading the model/textures. Diagnostics distinguish that stage; another fresh probe and the final terminal runs succeeded. An orphan from a previously closed Aster was also found and stopped. These observations do not establish a single root cause for every timeout. The local renderer now has one bounded startup retry and retains the first failure in its diagnostics.

---

# Live2D startup recovery — 2026-09-24

Aster 0.2.1 adds explicit loading stages, browser-exit detection, private diagnostic records, and `/pet retry`. The renderer bypasses proxy settings for its loopback asset connection. A renderer worker failure is reported instead of leaving the initial loading message indefinitely.

- **19 Rust tests passed**, including immediate browser-exit diagnostics and a missing-renderer failure that replaces the loading status. Formatting, Clippy with warnings denied, and the release build passed.
- A real PTY test deliberately failed the first browser launch, verified the visible error and saved diagnostic, issued `/pet retry`, and received **10 distinct native iTerm image frames** from the actual nine-texture model. It exited with status 0 and made no API calls. Local evidence: `.aster/qa/live2d-startup-recovery.json`.
- The full keyboard-driven terminal test also passed with Live2D enabled: project instructions, slash completion, approval before writing, independent JSON checks, session fork/resume/export, resizing, and clean exit. Its driver now waits for a modal to close before sending the next command instead of relying on a fixed delay during model loading.
- The user's affected session previously reported `Opening her room…` with zero frames. After restarting into 0.2.1, its own diagnostic recorded `Live2D · connected`, the first captured frame, and all nine model textures. The user then confirmed that 弄玉 was **visible and moving in iTerm**. The old build discarded browser stderr, so the original failure's exact cause was not established.
- The GitHub repository is public. All 59 historical Git blobs were checked before the visibility change; no configured credential or token-pattern match was found. Local model assets and SDKs remain excluded.

---

# Rust rebuild validation — 2026-09-24

Aster 0.2 uses the Rust application as its default launcher. The older Python results below are historical; they are not counted as Rust coverage.

- `cargo test`: **17 tests passed**. Coverage includes real demo tool execution, permission before writes, plan-mode enforcement, cancellation before side effects, typed JSON checks, path and symlink boundaries, SSE thinking/tool-block preservation, truncated streams, private session persistence, context checkpoints, named forks, Unicode editing and compact/wide layouts.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and the optimized release build passed.
- A real PTY drove slash completion, AGENTS.md inspection, named sessions, file-write approval, exact JSON verification, conversation fork/resume, Markdown export, 132×42 → 80×24 → 132×42 resizing, and zero-status exit.
- The same PTY flow passed with the native iTerm inline-image path enabled. The Rust renderer loaded the actual nine-texture 弄玉 model, captured distinct idle/speaking frames, and emitted inline PNG image packets. Renderer processes and their temporary profiles were absent after exit.
- Kitty graphics packet construction is unit-tested. Terminal-specific GUI rendering in Ghostty and Kitty has not been manually inspected. The full-layout preview comes from the actual Ratatui draw function and an actual model frame, rendered to SVG/PNG for visual inspection.
- A fresh zsh login from `/tmp` resolves `aster` to version **0.2.0**. The launcher preserves the caller's project directory and runs the Rust binary.

## Live model validation

The successful live TUI run `0da59eeddd14` used **MiniMax-M2.7**, **3 model turns**, **3 tool calls**, **3,647 input tokens**, and **259 output tokens**. The test inspected the permission prompt before approving the expected file write. The resulting `result.json` matched this object exactly and passed the Rust `json_equals` check:

```json
{"language":"Rust","companion":"弄玉","proof":"jade-486"}
```

The session was saved and the terminal exited with status 0. Evidence is local in `.aster/qa/rust-minimax-e2e.json`; the complete private provider conversation is in the ignored QA session store.

An earlier development run `dcf07cdcf985` wrote the correct file, but its tool check failed: the model supplied JSON text to an ambiguous schema that accepted any JSON type. The checker correctly rejected string-versus-object equality. The schema now explicitly asks for serialized JSON text and the checker parses it before comparing JSON values, preserving boolean/number/string distinctions. A regression test covers this. The test driver's separate `/var` versus `/private/var` path comparison was also corrected. The earlier run remains recorded as a failed check; it used **7,627 input tokens** and **670 output tokens** across **6 model turns**. No automatic provider retries were added.

Total reported live usage for the Rust rebuild: **11,274 input tokens and 929 output tokens**, across these two explicitly invoked test turns. This is not a model benchmark or a currency-cost claim.

## Local visual evidence

- `.aster/qa/model/idle.png`, `speaking.png`, and `renderer.json`: Rust-owned renderer probe.
- `.aster/qa/aster-rust.svg` and `aster-rust.png`: full workbench preview.
- `.aster/qa/rust-terminal-e2e.json`: terminal workflow and graphics-path checks.

Model assets and SDK files are read from the existing desktop-pet directory and were not copied into source. API keys, session data, private profiles, generated previews and build outputs are excluded from Git.

---

# Historical Python prototype validation

## Terminal interface — 2026-09-24

**26 tests pass** in the project virtual environment, including the original 17 engine tests and 9 terminal/backend tests. Terminal coverage includes keyboard launch, explicit failure display, a checkpoint branch that preserves its parent, invalid-limit feedback without a model call, compact layout and resizing, in-flight cancellation, custom manifest execution, control-character sanitization, and workspace-verified dashboard attachment.

Two separate real pseudo-terminal tests also passed. They launch the executable through the operating system, send keyboard input, parse the screen, inspect a produced file, inspect checks, export JSON, resize from 132×42 to 80×24 and back, and verify a clean zero-status exit. One used the deterministic demo; the second used the live MiniMax provider. They are recorded in `.relay/qa/terminal-demo.json` and `.relay/qa/terminal-live.json`.

The new live terminal run `8d1489220b21` passed its configuration check using **3 model turns, 2 tool calls, 1,720 input tokens, and 283 output tokens**. It was launched and inspected through the TUI, not invoked directly through the engine. The exported `service.json` was independently compared with all five expected fields. The dashboard remained available after the terminal exited.

Visual QA reviewed the full-width terminal view, new-run form, and compact layout. Final SVG previews are in `.relay/qa/relay-terminal.svg`, `relay-terminal-checks.svg`, and `relay-terminal-compact.svg`. The terminal companion is a colored half-block rendering of the existing model portrait, not a Live2D animation. No desktop-pet assets were modified or copied into source.

The code and generated trace/preview files were scanned again for the configured credential: no copies were found. The terminal dependencies are isolated in the project's `.venv`.

---

Verified locally on 2026-09-24 with the configured MiniMax endpoint. These are development checks, not a model benchmark.

## Harness tests

`python3 -m unittest discover -s tests -v`: **17 tests passed**.

Coverage includes artifact verification, deliberately wrong output, checkpoint restoration without changing parent files, turn/tool/output limits, cancellation during an API request, path traversal and symlink escapes, restricted arithmetic, strict tool arguments, malformed task definitions, truncated responses without retries, full MiniMax content-block round trips, unverified status without checks, local HTTP boundaries, and exclusive data-directory access.

`node --check relay/static/app.js` and Python compilation passed. Browser checks covered new-run submission, checkpoint branching, run history, task/tool navigation, output-file inspection, and visible verification outcomes. The browser console reported no errors or warnings. The local export endpoint returned the recorded task, events, checks, and files without private checkpoint messages.

## Live MiniMax runs

Model: `MiniMax-M2.7`. Every row below used the real provider and real workspace tools.

| Run | Task | Result | Model turns | Tool calls | Input tokens | Output tokens |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| `376ff5606626` | Sales report | Failed one check | 5 | 5 | 3,761 | 726 |
| `54e74b29b604` | Branch of the sales run at checkpoint 1 | Failed one check | 3 | 7 | 2,744 | 686 |
| `1e0522e790de` | Service config repair | Passed | 3 | 2 | 1,722 | 276 |

Total reported usage: **8,227 input tokens and 1,688 output tokens** across 11 model requests. No claim is made about currency cost or subscription quota accounting.

Both sales runs wrote total revenue `525` but incorrectly selected `Keyboard` as the top product. Paid revenue by product is Keyboard `160`, Mouse `125`, and Monitor `240`. The independent checker rejected both reports. The branch restored the earlier workspace and conversation; it did not guarantee a better model answer. Both failures remain in local history.

The config run read `service.json`, changed `retries` to `3`, `timeout_seconds` to `30`, and `debug` to `false`, preserved `service` and `region`, and passed exact JSON comparison.

The original sales manifests used a full-content containment check for source preservation. New sales tasks now use exact text equality; the prior run manifests were intentionally preserved. This change does not affect the independently detected incorrect product.

## Credential check

The project-local `.env` has mode `0600` and is excluded by `.gitignore`. A scan found no copy of its token in source files, tests, examples, or saved run records. The browser only receives a configured/not-configured flag. Existing app credentials/settings were not changed.

## Current scope

This prototype supports text-file tasks and arithmetic, with one active run at a time. It has no shell execution, browser automation tool, MCP adapter, durable job queue, or multi-user authentication. Limits bound work but are not a currency budget. Stopping locally cannot refund or cancel an already submitted provider request.
