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

## Context size and compaction

An expanded user message is limited to 80 KB. Before sending a model request, Aster rejects saved messages plus system context above 400 KB. Use `/context` to inspect, then `/compact` or `/new` to reduce context. `/compact [note]` first archives the complete session privately, including original provider blocks. It retains up to four recent exchanges within 64 KB, never separating a tool call from its result or changing a retained thinking signature. A single oversized or incomplete exchange can be archived in full instead. The resulting provider context is bounded to 128 KB.

The local checkpoint includes excerpts of the original request, recent user requests, the latest recorded task state, and cumulative file-tool read/write paths (at most 128 of each). Assistant excerpts are marked as claims. The optional note is at most 2 KB and carries forward on later compactions; a new note replaces it. The checkpoint is an extract of recorded history, not a model-authored summary, and excerpts may be truncated. It makes no API request.

`/checkpoint` opens the retained history beside 弄玉 and shows the before/after byte counts plus archive ID. `/restore ID` opens the exact archived provider context as a new conversation. It preserves the compacted conversation, clears copied follow-up queues and never rolls back project files. An oversized restored context may need another compaction before sending a model request. Older sessions load without checkpoint metadata.

Private archives live under the session store's `archive/` directory. They contain full provider content and are excluded from the repository. A failed archive/save leaves the active in-memory conversation intact.
