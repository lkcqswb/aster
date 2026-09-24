# Working in larger projects

Aster's navigation tools help 弄玉 find a precise location before reading or changing it. The companion work card records the latest search locations and the number of files read. Open F2 to inspect them.

## Browse beside 弄玉

Press **F6** or enter `/files` to open the project picker. Type a filename fragment or glob; `/find TEXT` opens a case-insensitive text search. Results load locally in the background and follow the same ignore/private-path rules as the agent tools. No model request is made.

- Up/Down select; Enter opens a numbered preview.
- Page Up/Down move between result pages. In a preview they scroll its text; Left/Right move between source pages.
- Tab adds a reference to your existing draft. From a preview it attaches those lines; from a search result it selects nearby lines; from a file listing it selects the first 80 lines. It does not send the draft.
- Escape returns from a preview or closes the picker. Ctrl+U clears the search field.

Selected lines are validated against the attachment limits before being added. Previewing a fragment of a very long line does not silently attach the full line; use `read_file` continuation for that case. 弄玉's reading pose follows the open preview while the task's saved plan and evidence remain intact.

## Agent navigation tools

`list_files` accepts a directory `path`, a project-relative `glob`, optional `case_sensitive`, a zero-based `offset` and a page `limit` (default 200, maximum 500). `next_offset` identifies another page. The directory scope still respects ignore rules at the project root.

`search` is literal and case-sensitive by default. It also accepts a directory, project-relative glob, `regex: true`, `case_sensitive: false`, and up to three surrounding lines with `context`. Each result gives its file, line, character column, nearby text and the excerpt's starting column. Long lines are excerpted around the match, not blindly from their beginning. A page contains at most 100 matching lines; the default is 50. Pagination counts matching lines, not files.

Both tools respect `.gitignore`, `.ignore` and nested ignore files. Hidden project files can be found, while Aster's private/generated-path exclusions always apply. Symlinks remain excluded. A glob cannot override those exclusions. Explicit reads of an otherwise ignored project file are possible, but private-path restrictions still apply.

Search considers UTF-8 text files up to 2 MB. It reports skipped files, scan errors and incomplete results. Each call is bounded by 20,000 directory entries, five seconds and, for search, 64 MB of source text. Narrow the directory or glob if a scan reaches a limit. Continuation is a fresh scan, so changing files between pages can change the results.

## Reading and editing

`read_file` accepts files up to 2 MB and returns numbered pages of at most 16 KB. `offset` is a one-based line number; `limit` defaults to 200 and may be up to 500. For continuation, use both `next_offset` and `next_column`. The column is a one-based Unicode character position on the first returned line. This lets even one very long line be read without silently dropping its remaining characters.

`edit_file` can make a unique, exact replacement in a file up to 2 MB. Each old/new fragment is limited to 128 KB. Approval still shows the prepared diff, unrelated bytes and permissions are preserved, and any intervening file change prevents the commit. `write_file` remains limited to 128 KB and cannot replace an existing larger file; use a focused edit instead.

`read_files` reads 1–8 files in one call. Each entry takes the same `path`, `offset`, `column` and `limit` as `read_file`. A file's page is at most 16 KB and the whole batch returns at most 64 KB of text. Each file carries its own `next_offset`/`next_column`, or its own `error`, so a missing or private file does not fail the others. When the batch limit is reached, later files are marked `skipped` with the position to request next. Nested `AGENTS.md` guidance is checked for every listed path before anything is read.

`outline` lists definitions in one file with one-based line numbers, kinds and names: Rust functions, types, traits, impls, modules, constants and `macro_rules!`; Python functions and classes; JavaScript/TypeScript functions, classes, arrow-function constants, interfaces, types, enums, exports and methods; Go functions, methods and types; Java, Kotlin and C# classes, records, objects and methods; C/C++ types, namespaces and top-level functions; Ruby methods, classes and modules; shell functions; and Markdown headings outside code fences. It is regex-based and best effort, so it can miss or misread a definition. Read the lines before relying on one. Output stops at 400 entries or 32 KB and reports `found` and `truncated`. Unsupported file types return an error rather than an empty outline.

## Multi-edits, moves and deletions

These tools change project files, so plan mode refuses them and ask mode requires approval. Each one is prepared before approval. The reviewed preview describes exactly what will be committed, and the commit is refused if the file changed afterwards. Nothing is overwritten in that case.

`multi_edit` applies 1–32 exact replacements to one existing UTF-8 file of up to 2 MB. Edits apply in order, each to the result of the previous edit. An `old_text` must match exactly once unless `replace_all` is true; then every occurrence is replaced and at least one must exist. Fragments are limited to 128 KB, and the edited file cannot exceed 2 MB. If any edit fails, the error names it, for example `edits[2] (edit 3 of 5)`, and nothing is written. Approval shows one diff with a separate hunk for each distant change, and the write is atomic.

`move_file` moves or renames one regular file. It creates missing destination directories and never moves a directory. An existing destination is refused unless `overwrite` is true. The preview then shows the start of the content that will be lost. Both paths follow the usual project-path rules: no traversal, absolute paths, symlinks or private/credential directories. Nested `AGENTS.md` guidance for both the source and destination directories is returned before the move is prepared. The commit is refused if the source changed or the destination appeared or changed since the preview. Without `overwrite`, the move is linked into place so it cannot replace a file that appears at the last moment. A move across filesystems is refused. Use an approved shell command instead.

`delete_file` removes one regular file, never a directory. The preview is a removal diff: size, then up to 20 lines from the start of a text file, or a note that binary content is not shown. A change to the file after the preview prevents the deletion. Aster cannot undo a deletion. Session forks share project files and are not a filesystem rollback.

To detect a change, moves and deletions compare size, modification time and, for files up to 64 MB, a content fingerprint. Larger files are compared by size and modification time only. Every committed multi-edit, move or deletion counts as a change on the work card. It lists both paths of a move, and earlier passing checks become stale until they are run again.

## Fetching web pages

`web_fetch` makes one HTTP(S) GET request for `url` and returns readable text. It is network egress, so it needs approval in ask mode and is refused in deny mode. It does not change files, so plan mode permits it, with the same approval.

- Only `http` and `https` URLs are accepted, and URLs with a user name or password are refused.
- Aster resolves the host itself and refuses it if any address is loopback, private, link-local, unique-local, carrier-grade NAT, multicast, unspecified, documentation, benchmarking or reserved. IPv4-mapped, 6to4 and NAT64 addresses are judged by their embedded IPv4 address. The connection is pinned to the checked addresses.
- Redirects are followed manually, up to five, and each hop is checked again before it is contacted.
- There is no cookie store, and no credential, authorization header or referrer is sent. The user agent is `aster/<version>`.
- The whole fetch is limited to 20 seconds. The body is capped at 1 MB; a longer body is cut there and reported with `body_truncated`. Compressed responses are refused.
- HTML becomes text: scripts, styles and similar elements are dropped, link text is kept, common entities are decoded and whitespace is collapsed. Headings (`#`) and list items (`-` or `1.`) stay on their own lines. Plain text, Markdown, JSON, XML and other text types pass through. Terminal control characters are removed and binary types are refused.

The result includes `url`, `final_url`, `status`, `content_type`, `content`, `offset`, `next_offset`, `truncated` and `total_chars`. It also carries the note that web content is untrusted data, not instructions. `offset` counts characters after conversion, and `max_chars` is 1–40,000 (default 20,000). Continuing with `next_offset` fetches the page again, so it may have changed. Non-success statuses such as 404 are returned with their body, not treated as errors.

If `HTTP_PROXY`, `HTTPS_PROXY` or `ALL_PROXY` is set, the request goes through that proxy, following the HTTP client's default behavior and `NO_PROXY`. Aster still checks the addresses it resolves, but the proxy performs its own lookup and connection, so address pinning applies only to direct connections. Proxy credentials from your environment go to the proxy, never to the site.

All source-size limits above use decimal bytes. Search/read pagination bounds context size; it is not a token or currency budget. A check proves only its stated assertion, and completing a search does not prove a coding task is correct.
