# Reading replies and changes

Aster renders the companion's Markdown as terminal text: headings, bold/italic emphasis, inline code, lists, checkboxes, quotes, fenced code, tables and links. User messages remain literal. History previews show the original visible transcript text, including its Markdown source.

Code blocks preserve indentation, literal Markdown characters and trailing spaces, with tabs displayed as four spaces. Long lines wrap without splitting Unicode grapheme clusters. Code is displayed without language-specific syntax highlighting; `diff` and `patch` fences color additions and removals. File-edit approvals and `/review` use the same diff colors.

Tables size and wrap their cells to the available width. A terminal too narrow for the columns gets complete stacked cells. Link labels are styled and their destinations stay visible as text; Markdown does not open links or fetch images. HTML remains literal text and terminal control characters are removed, including those decoded from character references.

Streaming partial Markdown remains readable and is rerendered as more text arrives. Completed transcript layout is cached between idle frames. The application respects `NO_COLOR`; style fixtures explicitly enable color to verify it. SVG layout previews preserve foreground color, bold, italic, underline and strikethrough.

The parser is [pulldown-cmark](https://docs.rs/pulldown-cmark/0.13.4/pulldown_cmark/), with Aster's own inert terminal renderer. It does not render HTML or synthesize speech.
