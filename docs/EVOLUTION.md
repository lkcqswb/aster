# Aster workbench evolution

The companion should help you make decisions and follow real work. Animation alone is not useful task support.

## References

- [Pi agent runtime](https://github.com/earendil-works/pi/tree/main/packages/agent): typed lifecycle events, steering at tool boundaries, follow-up messages.
- [Pi extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md): tools, commands, questions, persistent state, event interception and custom terminal UI.
- [Pi compaction](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/compaction.md): bounded retained history, intact tool/result pairs, cumulative file tracking and recoverable context. Aster uses local excerpts here, with no summarization request.
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

## Delivered in 0.11

A local, asynchronous file/search picker beside the companion, numbered source previews, result/source pagination and validated references added to the existing draft. The reading pose follows the open preview; browsing does not start a model request or replace task evidence. Selected context can come from source files up to 2 MB while excerpt limits remain bounded.

## Delivered in 0.12

Plans, diffs, command output, checks and files remain available while a decision is pending. The companion reads with you; the pending decision remains visible in her work card. Closing inspection restores the original answer draft or approval, while cancellation and redirection remove obsolete decisions. File attachments go into the composer without answering or submitting. A newer external file edit still invalidates the prepared approval.

## Delivered in 0.13

Clicking the companion or pressing F1 opens searchable local actions. Command output and checks needing attention appear early; pending decisions can be resumed, inspected, redirected or stopped. Mouse selection and the scroll wheel work in the local views. The menu refreshes as work finishes so obsolete stop/redirect actions disappear. Opening controls preserves the composer and makes no model request. Choosing a skill or prompt prepends its invocation to the existing draft. Session and resource filters accept pasted text.

## Delivered in 0.14

Recoverable local context checkpoints with a byte budget, complete tool/result boundaries, user notes, original-request excerpts and cumulative file-tool history. A single oversized exchange can now be compacted. Restore creates a new conversation with exact archived provider blocks and leaves files untouched. The companion action menu exposes the checkpoint; model-request counts exclude a locally rejected oversized request. A real MiniMax continuation recovered the saved contract, reread current source, wrote with approval and passed an exact JSON check.

## Delivered in 0.15

Searchable visible conversation history with role filters, full entry previews, jumps to earlier entries and a return-to-latest shortcut. The draft and pending decisions survive inspection; provider thinking blocks are excluded. Transcript layout is cached between idle frames and keeps a scrolled viewport steady while messages arrive, allowing Live2D animation to continue without reformatting all earlier entries.

## Delivered in 0.16

Markdown replies with styled headings, emphasis, inline code, nested lists, task markers, quotes, fenced code, tables and visible link destinations. Unicode graphemes and code spacing survive wrapping; narrow tables use stacked cells. Approval/review diffs distinguish additions and removals. Terminal controls remain inert, and SVG previews retain text styling. The actual animated terminal verified colors, a real approved edit and an independent passing check.

## Delivered in 0.17

Local project task discovery and explicit task configuration, an F8 picker beside the companion, inspect-before-run controls and the existing approval/output/cancellation pipeline. Task browsing preserves pending decisions and drafts; execution needs no model request. Headless turns return a failing status for unresolved failed or stale checks, while a repaired check can succeed and ordinary conversation does not imply verification.

## Delivered in 0.18

A keyboard that behaves like a text editor and a transcript that cannot strand you: ↑/↓ move within the draft and then through earlier requests, while reading uses bounded paging with a visible "newer lines" marker and Esc/Ctrl+End back to the latest. Readline word motion (including macOS Option-arrows, which previously typed letters), recoverable draft clearing, a session picker with activity and message counts, create/delete controls and a sensible default selection. A redesigned layout with a rounded composer that shows mode and run state, content-sized panels with their hints on the border, and a context meter.

Per-turn limits are configurable and higher by default. Auto-compact archives the full context and replaces older exchanges with a model-written summary from one bounded, tool-free request before a request would crowd the window, falling back to local excerpts without retrying. New file tools and a reworked Live2D presentation are described in their own documents.

Next: preserve queued work when a turn needs attention, and make resuming that work explicit.

## Delivered in 0.19

Public Companion API v1 with declarative model/layout/binding profiles, named parameter emotions, one-shot keyframe motions, timed gaze, reset, real parameter bounds and acknowledgement snapshots. The terminal accepts custom emotion/motion names and strengths. JSON Schema, TypeScript types, asset-free contract tests, local profile validation and actual-model interface/preview export support AI-generated designs. Models and SDK assets remain local. Capability introspection found and corrected two nonexistent smile parameter names in the old renderer.

## Delivered in 0.20

The 0.18 follow-up work on top of the Companion API: batch reads, outlines, multi-edits, moves, deletions and web fetch as tools; a models and API keys panel; a frame pipeline that renders at the portrait's pixel size at about 15 fps (JPEG for iTerm2 and cells, PNG for Kitty) with a sharper quadrant-cell fallback; no whole-screen blinks; menu actions that always lead somewhere. 弄玉's emotion and gesture are decided upstream: after each reply, one structured request to the conversation's provider chooses from her profile's names, and the choice goes through the Companion API for the renderer to confirm. Aster no longer infers them from reply text, events or idle time.
