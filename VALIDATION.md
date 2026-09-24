# Validation record

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
