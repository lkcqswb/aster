# Working in larger projects

Aster's navigation tools help 弄玉 find a precise location before reading or changing it. The companion work card records the latest search locations and the number of files read. Open F2 to inspect them.

`list_files` accepts a directory `path`, a project-relative `glob`, a zero-based `offset` and a page `limit` (default 200, maximum 500). `next_offset` identifies another page. The directory scope still respects ignore rules at the project root.

`search` is literal and case-sensitive by default. It also accepts a directory, project-relative glob, `regex: true`, `case_sensitive: false`, and up to three surrounding lines with `context`. Each result gives its file, line, character column, nearby text and the excerpt's starting column. Long lines are excerpted around the match, not blindly from their beginning. A page contains at most 100 matching lines; the default is 50. Pagination counts matching lines, not files.

Both tools respect `.gitignore`, `.ignore` and nested ignore files. Hidden project files can be found, while Aster's private/generated-path exclusions always apply. Symlinks remain excluded. A glob cannot override those exclusions. Explicit reads of an otherwise ignored project file are possible, but private-path restrictions still apply.

Search considers UTF-8 text files up to 2 MB. It reports skipped files, scan errors and incomplete results. Each call is bounded by 20,000 directory entries, five seconds and, for search, 64 MB of source text. Narrow the directory or glob if a scan reaches a limit. Continuation is a fresh scan, so changing files between pages can change the results.

## Reading and editing

`read_file` accepts files up to 2 MB and returns numbered pages of at most 16 KB. `offset` is a one-based line number; `limit` defaults to 200 and may be up to 500. For continuation, use both `next_offset` and `next_column`. The column is a one-based Unicode character position on the first returned line. This lets even one very long line be read without silently dropping its remaining characters.

`edit_file` can make a unique, exact replacement in a file up to 2 MB. Each old/new fragment is limited to 128 KB. Approval still shows the prepared diff, unrelated bytes and permissions are preserved, and any intervening file change prevents the commit. `write_file` remains limited to 128 KB and cannot replace an existing larger file; use a focused edit instead.

All source-size limits above use decimal bytes. Search/read pagination bounds context size; it is not a token or currency budget. A check proves only its stated assertion, and completing a search does not prove a coding task is correct.
