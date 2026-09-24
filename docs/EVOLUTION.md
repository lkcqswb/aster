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

Next: stronger live command feedback, companion-guided recovery, and renderer startup/resource reliability.
