# Aster workbench evolution

The companion should help you make decisions and follow real work. Animation alone is not useful task support.

## References

- [Pi agent runtime](https://github.com/earendil-works/pi/tree/main/packages/agent): typed lifecycle events, steering at tool boundaries, follow-up messages.
- [Pi extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md): tools, commands, questions, persistent state, event interception and custom terminal UI.
- [Pi sessions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sessions.md): resumable conversation context and branching.

These inform Aster's Rust implementation; Pi is not embedded or required at runtime.

## Delivery order

1. Reliable coding tools: exact edits, reviewable diffs, stale-file rejection and paginated reads.
2. Work state: structured plans, task-scoped checks, questions and live tool activity.
3. Companion interaction: a work card beside the live model, review/answer/stop controls, persistent attention when action is needed, and expressions tied to evidence.
4. User control: steer ongoing work, queue follow-ups, inspect context and reference files.
5. Project customization: discoverable skills and prompts, explicit reload and durable task context.

Every delivery must preserve existing conversations, run the relevant Rust and PTY tests, and distinguish deterministic demo coverage from live-provider evidence. Browser assets, keys, private session content and reference checkouts remain local. Successful checks do not erase failed checks from the record, and the companion must never imply a task was verified because the model said it was done.

## Delivered in 0.3

Exact edits with prepared diffs and stale-file rejection; numbered/paginated reads; structured plans; interactive questions; task-scoped evidence; companion work/review controls; decisions displayed alongside the animated model; one bounded local renderer startup retry. A real MiniMax code repair and an independent rerun of unchanged tests passed.

## Delivered in 0.4

Steering at tool boundaries; Ctrl+G to redirect pending decisions; durable follow-up queues; inspection, removal and explicit restart of saved messages; user decision time separated from active work limits. Skipped actions produce provider-compatible tool results without being counted as executed checks. A live MiniMax redirection test left the old file absent and verified the new file.

## Delivered in 0.5

A searchable skill/prompt picker beside the model, lazy skill and supporting-file loading, explicit file/range attachments, context inspection and bounded preflight checks. The work card tracks skills and references. Project guidance still applies and resources grant no tool permissions. A live task read a skill reference and an attached line to produce exactly verified JSON.

## Delivered in 0.6

Live command output, explicit stop/timeout/exit states, bounded head-and-tail capture, F4 and portrait-click access, direct local commands without a model request, and user-invoked recovery from failure. The companion keeps the latest output and actual status in her work area. Side panels clear the underlying transcript to avoid stray background text.

## Delivered in 0.7

Configurable runtime texture scaling with a 2048-pixel default, actual size diagnostics and loading progress. Native assets remain unchanged. Three profiles were checked for animation and visual quality; the default uses 75% fewer texture pixels.

## Delivered in 0.8

Signal-aware shutdown, session checkpoints on interrupted work, and shared ownership of renderer/command process groups. Real terminal tests stop idle animation, an unanswered approval and a running command; a headless cancellation returns a failing exit status. Terminal restoration, child cleanup and private renderer profile removal are checked.

## Delivered in 0.9

Check identities and edit revisions distinguish current failures, successful repairs, stale passes and earlier results. The companion uses that evidence for her reaction and status. F5 opens the underlying history beside her. Manual file checks update the same record and preserve valid tool/result pairs in the next model context without a model call.

## Delivered in 0.10

Project-aware file listing and search with ignore rules, scopes, globs, case control, regular expressions, context lines and honest continuation/scan limits. Large-source reads continue through long Unicode lines; focused edits preserve the rest of files up to 2 MB. The companion records actual discovered locations in her work card.

Next: a project picker beside the companion and direct navigation from search results.
