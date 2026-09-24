# Conversation history and transcript layout 0.15 — 2026-09-24

- **79 Rust tests passed**, formatting, strict Clippy and release build passed. Coverage checks Unicode/role search, appended history, exclusion of provider thinking, cached idle layouts, invalidation after changes, near-match previews, draft-preserving jumps and a stable viewport when new entries arrive.
- Real animated terminal with **1,200 visible entries**: opening and filtering history took **468 ms on this machine**. It read the full selected entry, jumped to it, returned to the latest messages, kept the composer draft and inspected history while preserving a pending question. **30 distinct frames**, zero API calls, exit 0. Evidence: `.aster/qa/history-e2e.json`. The initial driver expectation was corrected to tolerate a wrapped sentence; the displayed content was complete.
- The general terminal regression passed. The cache avoids reformatting unchanged transcript entries on idle animation frames; the timing above measures the fixture interaction, not a cross-machine performance guarantee.

# Recoverable context checkpoints 0.14 — 2026-09-24

- **75 Rust tests passed**; formatting, strict Clippy and release build passed. Coverage includes oversized single exchanges, exact retained tool/thinking blocks, repeated checkpoints, incomplete tool batches, failed writes, archive failure, cross-project restore rejection, private archive permissions and context preflight request counts.
- Animated terminal: **600,667 → 1,270 bytes**, 11 distinct frames, exact provider-context archive/restore, separate restored conversation, unchanged newer file state, zero API requests, exit 0. The fixture's initial macOS `/var` alias was corrected to the canonical project path before the successful run. Evidence: `.aster/qa/checkpoint-e2e.json`.
- **Live MiniMax-M2.7 continuation** from the compacted fixture recovered the original JSON contract without restating it in the new prompt, reread `proof.txt`, requested one reviewed file-write approval and passed `check_file json_equals`. An external JSON comparison matched `{"proof":"checkpoint-486","verified":true}` and the source stayed unchanged. **4 requests, 5,095 input / 327 output tokens**, no retry, exit 0. Session `c0ffee123456` in private `checkpoint-live-sessions-3f7ba808`; evidence `.aster/qa/checkpoint-live.json`. This proves the bounded fixture continuation, not lossless semantic summarization of arbitrary conversations.
- General terminal regression passed. Checkpoints contain local excerpts with explicit limits, not a model-written summary. Restoring provider context does not roll back project files.

# Companion actions 0.13 — 2026-09-24

- **68 Rust tests passed**, formatting, strict Clippy and release build passed. Coverage includes live menu availability, mouse hit targets, preserved decisions, pasted filters, skill selection retaining the existing request and compact layouts.
- The real animated terminal opened the menu by clicking the portrait, opened files and live command output with mouse clicks, returned to a question, stopped an unapproved write before any file existed, and cancelled a running shell command. The stop action disappeared after completion. **45 distinct frames**, resize, zero API calls and exit 0. Evidence: `.aster/qa/actions-e2e.json`.
- General terminal and context workflows passed. The final actual companion/action-menu preview was rendered and visually inspected: `.aster/qa/aster-actions.svg` and `.png`. Selected actions use jade, descriptions are muted, and the portrait remains visible beside the menu.

# Decision inspection 0.12 — 2026-09-24

- Rust: **65 tests passed**; formatting, strict Clippy and release build passed. Tests keep question/approval channels open across view switches, reject approval keystrokes in an inspection, preserve draft answers, and remove pending decisions on cancellation or redirection.
- The actual animated terminal inspected files during a question, attached a source range without answering, reviewed every panel during approval, then completed a real write/edit/check. A second run changed the target externally while the edit approval was being inspected: accepting that old approval left the newer bytes intact and the final check failed honestly. **93 distinct frames**, zero API calls, exit 0. Evidence: `.aster/qa/decision-review-e2e.json`.
- The general terminal flow passed: slash commands, AGENTS.md, approvals, independent file check, session fork/resume/export, resizing and clean exit. The first new driver attempt matched an old question in the transcript; it was corrected to wait for the actual decision heading and controls before answering.

# Companion project picker 0.11 — 2026-09-24

- **62 Rust tests passed**, plus formatting, Clippy with warnings denied and release build. New cases cover replacing an in-flight query, case-insensitive filename filtering, result/source pagination in both directions, large-file preview ranges, and attaching to an existing draft without submitting it.
- The actual animated terminal flow opened F6, filtered a filename, read and paged through its preview, attached lines 41–80 while preserving the draft, searched a larger file, previewed line 8,001 and attached lines 7,997–8,002. Only an explicit Enter submitted that context. Source files stayed unchanged; **34 distinct frames**, resize and exit 0 were observed. Evidence: `.aster/qa/navigator-e2e.json`. No API requests were made.
- The context/skills and general session/approval/fork/export terminal regressions passed. The actual file picker and model were rendered and visually inspected in `.aster/qa/aster-files.png`.
- Source previews run on cancellable background workers. The selected excerpt remains bounded to 16 KB; raising the source-file limit to 2 MB does not attach an entire large file.

---

# Project navigation and larger sources 0.10 — 2026-09-24

- **58 Rust tests passed**, covering root/nested ignore rules during scoped searches, exact list and matching-line pagination, regex/case/context controls, invalid ignore diagnostics, cancellation, binary/oversize skips, symlink/private-path exclusions, case-insensitive credential names and focused large-file edits. Formatting, Clippy with warnings denied and release build passed.
- A 180 KB Unicode line was reconstructed across numbered read pages without losing a character. A focused edit in a larger source preserved all unrelated bytes; whole-file replacement and a stale second commit were rejected.
- One live MiniMax task found the target at line **8,001** in a **352,068-byte** catalog, read at most five nearby lines, made the approved exact edit and checked the full changed record. A separate full-file comparison and SHA-256 comparison proved every unrelated byte stayed unchanged. Session `24d6dce7ebb3`: **7 provider requests, 9 tools, 13,099 input tokens, 1,132 output tokens**, exit 0. Evidence: `.aster/qa/project-live.json`. No provider retry or shell command was used.
- The context/skills terminal regression and general session/approval/fork/export/resize regression passed. The general driver's command submission now waits for the composer to receive and submit the command, and session switching waits for the picker to close. This fixes races that previously consumed the beginning of the next command; those earlier attempts are not passing results.
- Navigation reports skipped/incomplete work and obeys explicit size/time/entry limits. Pagination is a fresh scan, not a frozen filesystem snapshot. The work card records actual discovered paths and counts.

---

# Evidence and companion review 0.9 — 2026-09-24

- **50 Rust tests passed**, plus formatting, Clippy with warnings denied and release build. New cases cover failed-then-passing exact checks, weaker assertions that cannot erase a failure, stale passes after edits, later regressions, old session records, and valid provider pairs for manual checks.
- The animated terminal workflow recorded a failure, repaired it, retained both results, made another edit, displayed the resulting stale check, and refreshed it with an exact local recheck. F5 history and narrow/wide resizing passed with **91 distinct frames** and exit 0. Evidence: `.aster/qa/evidence-e2e.json`. No API requests were made. The general session/approval/fork/export terminal regression also passed.
- This test exposed an unnecessary cursor-position query during full-screen redraw: a missed reply ended Aster with a terminal error. Redraw now invalidates the full-screen buffer without that query. The regression explicitly rejects cursor queries and resizes while the checks panel is open.
- The test driver now strips complete inline-image packets before parsing terminal text and waits for the full panel transition before sending the next command. Earlier attempts caught delayed input and a command sent during redraw; these are not counted as passing runs.
- Evidence freshness covers Aster's file-tool edits. Arbitrary shell mutations and external file changes are not automatically detected; a passed command is evidence only for that command.
- The actual checks panel and model were rendered and visually inspected. Evidence is presented as readable assertions/outcomes; original provider results remain in private session context.

---

# Shutdown and process ownership 0.8 — 2026-09-24

- **45 Rust tests passed**, plus formatting, Clippy with warnings denied and the release build. The general keyboard/session/approval/fork/resize PTY regression passed.
- Real owned-process signal tests passed for animated idle (`SIGHUP`), a pending approval (`SIGTERM`), an active command (`SIGTERM`) and a headless command (`SIGTERM`). The terminal cases saved state, restored the alternate screen and exited in **0.378, 0.237 and 0.212 seconds** in this run. Headless cancellation saved a stopped result and returned exit 1.
- All nine observed renderer processes ended and its one private profile was removed. The shell and its background child ended in both command cases. The unapproved command never started; no delayed completion marker appeared. Evidence: `.aster/qa/shutdown-e2e.json`. No API requests were made.
- Signal handling is cooperative; it does not promise cleanup after `SIGKILL`, a machine crash, or cancellation of an already submitted provider request.

---

# Renderer texture profiles 0.7 — 2026-09-24

- **45 Rust tests passed**, including validated texture bounds and the private settings endpoint's host/path restrictions. Formatting, Clippy with warnings denied and the release build passed.
- Native 4096, balanced 2048 and smaller 1024 profiles each loaded all nine textures on the first attempt and captured at least nine changing frames. Their observed probe times were **4.840, 4.496 and 4.131 seconds** in this one local comparison. This is not a general startup benchmark.
- The default 2048 profile rendered **37,748,736 texture pixels versus 150,994,944 native pixels**, a **75% reduction**. The 1024 profile rendered 9,437,184 pixels, a 93.75% reduction. These are texture-pixel counts, not measured total process memory. Source model, metadata and texture hashes were identical before and after all probes. Evidence: `.aster/qa/texture-profiles.json`.
- The actual 420×620 portrait at all three settings was visually inspected. The 2048 setting retained the visible face, hair, ornaments and clothing detail at the terminal's rendering size; native texture mode remains available. Assets and vendor SDK files were not modified or committed.
- The full companion decision/edit/check/review PTY workflow passed with **40 distinct frames**, including resizing and clean exit. The deliberately failed startup test exhausted the single automatic retry, then `/pet retry` recovered with **10 distinct frames**, nine textures and exit 0. No API requests were made for this update.
- Startup diagnostics now record rig/texture progress and original/rendered dimensions. The intermittent historical startup timeout has not been assigned a proven root cause.

---

# Live command work and recovery 0.6 — 2026-09-24

- **43 Rust tests passed**, including output arriving before process exit, exact failure status/stderr, distinct cancellation and timeout, bounded first/last capture, and direct local commands with no key or model request. Formatting, Clippy with warnings denied and the release build passed.
- The animated PTY command workflow passed with **92 distinct Live2D frames**: output was visible before exit, exit 7 and stderr were saved, timeout was distinguished from stop, the interrupted command ended, and Aster exited with status 0. Evidence: `.aster/qa/command-live2d-e2e.json`. Queue, context and general terminal regressions also passed.
- `/run python3 -m unittest -v` reproduced three failing tests with **zero model requests**. `/recover` then used MiniMax to plan, inspect, make an approved exact edit and run the approved test command. The three unchanged tests passed, including an independent rerun. Session `fd041b063014`: **5 provider requests, 7 total tools including the initial local command, 10,677 input tokens, 722 output tokens**, exit 0. Evidence: `.aster/qa/agent-recovery-live.json`.
- An earlier recovery test was interrupted before its edit was approved because the test driver inspected a partially drawn approval. That saved attempt used **2 requests, 2,947 input tokens, 317 output tokens**. The driver now waits for the complete expected edit/command before approving. Total live usage across these two attempts: **13,624 input tokens, 1,039 output tokens**. No provider transport failure or truncation was automatically retried.
- Headless direct-command checks returned exit 0 for success and exit 1 for failure, both with zero model requests. The command panel was rendered from the real draw function with the actual model and visually inspected. Failed-check attention now persists in the companion until new work replaces it.
- Other test-driver fixes distinguish panel content from old transcript text and strip panel borders when checking output. A previous apparent quit hang was the driver sending its command before the panel opened; the corrected animated flow exited cleanly.
- Three independently started renderer probes completed on their first attempt in **5.15, 5.01 and 5.03 seconds**, each capturing changing frames. They loaded the original nine 4096-pixel textures. This does not establish the cause of intermittent slow starts; resource optimization is still planned.

---

# Skills and project context 0.5 — 2026-09-24

- **39 Rust tests passed**, with YAML metadata/precedence, lazy skill loading, explicit-only skills, supporting-file boundaries, selected-line attachments, scoped project rules and readable-transcript/provider-context separation. Formatting, Clippy with warnings denied and the release build passed.
- The offline PTY context test passed: searching/preparing a skill without a model call, explicit invocation, exact attached lines, context inspection, prompt expansion, reload, compact resize and zero-status exit. The general session/command/approval/fork terminal regression also passed. Evidence: `.aster/qa/context-e2e.json`.
- One live MiniMax task loaded a skill with `read_skill`, read `refs/proof.txt` through that tool, combined it with line 2 of an attached input, and wrote exactly `{"value":7,"proof":"nongyu-context-486"}`. Only the expected write was approved; the JSON check passed. Session `8b0bb4133255`: **5 model requests, 4 tools, 4,283 input tokens, 435 output tokens**, exit 0. Evidence: `.aster/qa/context-live.json`.
- An earlier attempt caught a sentence-punctuation bug in `@input.txt:2.` during local preparation: **zero tools and zero provider tokens**. The parser and its regression test were corrected before the successful live task. No provider failure was retried.
- A read-only `--preview-panel` option supports actual-layout SVG inspection of skills, prompts, context, work and review. Model assets and all local QA/session output remain excluded from Git.

---

# Steering and durable messages 0.4 — 2026-09-24

- **34 Rust tests passed**, including steering before execution, cancelling an unapproved write, preserving provider tool/result pairs, direction-box cancellation, saved follow-ups across stop/resume, and excluding unexecuted actions from check evidence. Formatting, Clippy with warnings denied and the release build passed.
- The keyboard queue test passed: Enter steers, Alt+Enter schedules a follow-up, a stopped queue survives restart, restart does not run it automatically, and `/next` consumes its message once. No API calls. Evidence: `.aster/qa/queue-e2e.json`.
- One live MiniMax task was redirected through **Ctrl+G** while `original.json` was awaiting approval. The original file was never created; only `final.json` was approved, its exact JSON passed `check_file`, and the direction was recorded once. Session `ad8fae1e7a0c`: **4 model requests, 2 executed tools, 2,474 input tokens, 309 output tokens**, exit 0. No provider retry. Evidence: `.aster/qa/steering-live.json`.
- The real companion workflow passed again with **40 distinct Live2D frames**, including animation during questions/approvals, the selected Chinese answer affecting the file, exact edits, checks, review and resizing. One startup needed the bounded local renderer retry. Evidence: `.aster/qa/companion-work-e2e.json`.
- PTY text snapshots now handle isolated wide-character continuation cells in the test emulator after resize. Failed companion tests request graceful application shutdown before forced termination.
- Human decision waits no longer consume the 180-second active work budget; each decision expires after 15 minutes without assuming an answer. These limits are not a currency budget.

---

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
