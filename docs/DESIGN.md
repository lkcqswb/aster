# Aster 0.2 — a room with 弄玉

The primary interaction is a conversation with 弄玉, an explicitly fictional companion who helps with real project work. Aster runs the agent loop, sessions, instructions, tools and terminal interface in Rust. The character is rendered from the user's own `.moc3`, textures and physics by the existing Live2D Cubism Web SDK in a private headless Chromium process. Frames are placed *inside* the terminal using native graphics. No separate companion window is required.

## References and decisions

- [OpenCode's TUI](https://opencode.ai/docs/tui/): discoverable slash commands, named persistent sessions, an on-demand session picker and a single composer.
- [OpenCode's rules](https://opencode.ai/docs/rules/): inspectable AGENTS.md project guidance. Aster explicitly lists the rules included in each turn and respects narrower directory guidance when tools access files.
- [Claude Code's interactive mode](https://code.claude.com/docs/en/interactive-mode): conversation first, concise tool summaries, escape to interrupt, and context controls.
- [Ratatui](https://ratatui.rs/): immediate rendering, responsive terminal layout, keyboard and mouse events.
- [iTerm inline images](https://iterm2.com/documentation-images.html) and [Kitty graphics](https://sw.kovidgoyal.net/kitty/graphics-protocol/): actual raster frames in iTerm2, Ghostty and Kitty. Character-cell rendering is only a compatibility fallback.

The visual language is ink, parchment and restrained jade: generous margins, quiet dividers, rounded low-contrast frames only where input happens (the composer and panels), and one footer that carries short hints and the context meter. Your messages carry a gold margin mark, 弄玉's replies a jade name, and tool activity collapses to one glyph-led row (✓ verified, ✗ failed, · done). The companion owns the right third of a wide terminal. On narrow terminals the portrait gets smaller instead of burying the conversation. Task controls stay beside the portrait when space permits; narrow layouts release graphics cleanly while showing a modal. F1 or a portrait click opens a restrained action list with a jade selection, muted descriptions, keyboard filtering and mouse activation.

## Character contract

Typing → attentive; model request → thinking; tool execution → working; streamed reply → speaking; explicit successful check → pleased; error → concerned. Idle breathing, eye blinking, subtle head motion, physics, a tap reaction, /look and /mood provide presence. Speaking animation is driven by text activity, not claimed audio lip sync. The character never claims to be a real person, hides no tool activity, and does not claim correctness before verification. Her body language follows actual run state.

## Scope

Sessions are local and private. Resume, rename, fork, export and delete are explicit commands; forks share the project filesystem and do not roll back files. Context compaction archives the complete conversation before replacing older exchanges with a summary. Since 0.18 that summary is normally written by the model in one bounded request, with local excerpts as the fallback, and it is always labeled as claims rather than evidence. File tools are bounded to the selected project. Reads omit credential/private directories. Writes and shell commands require approval unless the user explicitly selected allow mode. Plan mode prohibits both. Checks are independent assertions against saved files. No automatic API retries. Configurable budgets limit requests, tools, output tokens and active time per turn.

Model assets, SDK binaries, API keys, sessions and Chromium profiles are local dependencies, not repository contents. The older Python prototype stays available as historical source, but the `aster` entry point uses the Rust binary.
