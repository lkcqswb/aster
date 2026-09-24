# Project context and reusable workflows

Aster keeps a short user transcript and a separate private provider conversation. `/context` shows the current message count, content size, project guidance, skills used this turn and attached files. It does not expose private reasoning blocks. Content sizes are byte counts, not exact model token counts.

## Attach a file

```text
Explain @src/main.rs
Review @src/main.rs:10-40.
Read @{docs/design notes.md:1-80}
```

Only explicit references in the user's message are attached. Email addresses are not file references. Braces support spaces and literal punctuation in filenames. Without a range, Aster includes the first 200 lines. References are limited to eight files, 500 lines per range, 16 KB per excerpt and 48 KB combined. The source file may be up to 2 MB. File tools can read more pages later.

Nested AGENTS.md guidance is included before the attached file is used. Traversal, symlinks and credential/private paths are rejected. Invalid references stop preparation before any provider request. Steering can carry references too; if an attachment fails during a turn, the model receives an explicit failure rather than fabricated file content.

## Skills

A skill is a directory with a `SKILL.md` file:

```text
.agents/skills/repair-test/
  SKILL.md
  references/
    workflow.md
```

```markdown
---
name: repair-test
description: Diagnose and fix a reproducible failing test.
---

Read the relevant source and tests. Reproduce the failure before editing.
Read references/workflow.md for the project-specific procedure.
```

Use `/skills` to search the catalog beside 弄玉. Enter prepares an invocation in the composer; add your request and send it. F1 inspects the source without a model call. `/skills NAME` also inspects a skill directly.

```text
/skill repair-test fix the failing parser test
```

The model initially receives only skill names and descriptions. It calls `read_skill` to load instructions or supporting files. The companion's work card records the loaded skill; `/work` and `/context` preserve that trace. A skill can ask for a plan or an essential question, but it does not grant file or command permissions.

Aster discovers these locations, in order; later entries with the same name take precedence:

1. `~/.agents/skills`
2. `~/.config/aster/skills`
3. `PROJECT/.agents/skills`
4. `PROJECT/.aster/skills`

Names use lowercase letters, digits and hyphens, with up to 64 characters, and must match the containing directory. Descriptions are required and support YAML folded text. Skill files are limited to 32 KB. Discovery is bounded by depth and file count; malformed metadata and shadowed names appear in `/reload` diagnostics. Symlinks are excluded.

Set `disable-model-invocation: true` in the frontmatter to require explicit invocation. Supporting text files are resolved relative to the skill directory and limited to 128 KB. Skills are refreshed at each turn; `/reload` inspects current discovery. A turn keeps its initial catalog.

The repository includes an example in `examples/skills/repair-test`. Copy the directory into a project skill location to try it. These resources follow the lazy-loading pattern described in [Pi's skill documentation](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/skills.md); Aster does not depend on Pi.

## Prompt templates

Place Markdown files in `~/.config/aster/prompts` or `PROJECT/.aster/prompts`. Project templates take precedence. An optional YAML `description` appears in the picker. The filename supplies the command name.

```markdown
---
description: Review a selected change
---
Review $ARGUMENTS. Report concrete bugs with evidence. Do not edit files.
```

```text
/prompts
/prompt review the parser change
```

`$ARGUMENTS` is plain text substitution. No shell evaluation occurs. If the marker is absent, arguments are appended as the user's request. An example lives in `examples/prompts/review.md`.

## Context size, auto-compact and checkpoints

An expanded user message is limited to 80 KB. The footer shows an estimate of the next request's context as a share of the model's window (`ctx 42%`). The estimate is calibrated from the provider's own input-token count for the latest request; before the first request it assumes about three bytes per token. It is an estimate, not a tokenizer.

**Auto-compact.** Before each model request, including between tool rounds, Aster compares that estimate with `--auto-compact` percent (default 80) of `--context-window` tokens (default 200,000). When the context has reached the threshold and older exchanges exist, Aster compacts it before sending the request. It compacts at most twice per turn and never when nothing older could be archived. `/compact auto off` disables it for one conversation; `--auto-compact 0` disables it for the run. A request whose serialized context exceeds five bytes per window token (1 MB by default) is refused without sending.

**Checkpoints.** Automatic and manual compaction (`/compact [note]`) follow the same steps:

1. Choose the split. Up to four recent exchanges within 64 KB are retained intact. A tool call is never separated from its result, and retained thinking signatures are never changed.
2. Summarize the older part. 弄玉 makes **one bounded request with no tools**: a condensed transcript (private reasoning omitted, tool output clipped, at most 160 KB, keeping the beginning and the most recent work), the previous summary and your note. The requested sections are goal and intent, decisions, files and code, current state with evidence status, and next steps. The request is capped at 4,096 output tokens and 150 seconds, and it never streams into the conversation. Its tokens are added to session usage and it counts as a model request.
3. If that request fails, times out, is truncated or the key is missing, Aster falls back to local excerpts: the original request, recent user requests, the latest recorded task state and assistant excerpts marked as claims. The checkpoint records why. **There is no automatic retry.** The offline demo writes a visibly scripted summary locally and makes no API call.
4. Archive, then replace. The complete conversation, including original provider blocks, is saved privately under the store's `archive/` directory first. If saving fails, nothing is replaced. The new context contains the summary, a local record of files read and written by file tools, the note and the retained exchanges. It is bounded to 128 KB. When compaction happens mid-turn, a short continuation message restates the current request, so the model always answers a user message.

`/compact local [note]` makes the checkpoint from local excerpts only, with no request. The note is at most 2 KB and carries forward to later checkpoints; a new note replaces it. A model summary is a claim written by a model, not evidence: it can omit details. The summary text tells the model to re-check files before acting, and checks still need to be rerun.

`/checkpoint` shows the summary, whether it was automatic, its method (model, demo or local, with any fallback reason), the before/after sizes and the archive ID. `/restore ID` opens the exact archived provider context as a new conversation. It preserves the compacted conversation, clears copied follow-up queues and never rolls back project files. An oversized restored context may need another compaction before the next request. Older sessions load without checkpoint metadata and with auto-compact on.
