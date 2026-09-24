# Aster 0.2 — a room with 弄玉

The primary interaction is a conversation with 弄玉, an explicitly fictional companion who helps with real project work. Aster runs the agent loop, sessions, instructions, tools and terminal interface in Rust. The character is rendered from the user's own `.moc3`, textures and physics by the existing Live2D Cubism Web SDK in a private headless Chromium process. Frames are placed *inside* the terminal using native graphics. No separate companion window is required.

## References and decisions

- [OpenCode's TUI](https://opencode.ai/docs/tui/): discoverable slash commands, named persistent sessions, an on-demand session picker and a single composer.
- [OpenCode's rules](https://opencode.ai/docs/rules/): inspectable AGENTS.md project guidance. Aster explicitly lists the rules included in each turn and respects narrower directory guidance when tools access files.
- [Claude Code's interactive mode](https://code.claude.com/docs/en/interactive-mode): conversation first, concise tool summaries, escape to interrupt, and context controls.
- [Ratatui](https://ratatui.rs/): immediate rendering, responsive terminal layout, keyboard and mouse events.
- [iTerm inline images](https://iterm2.com/documentation-images.html) and [Kitty graphics](https://sw.kovidgoyal.net/kitty/graphics-protocol/): actual raster frames in iTerm2, Ghostty and Kitty. Character-cell rendering is only a compatibility fallback.

The visual language is ink, parchment and restrained jade: generous margins, almost no boxes, quiet dividers, one live status line. The companion owns the right third of a wide terminal. On narrow terminals the portrait gets smaller instead of burying the conversation. Modal controls temporarily cover the portrait and release graphics cleanly.

## Character contract

Typing → attentive; model request → thinking; tool execution → working; streamed reply → speaking; explicit successful check → pleased; error → concerned. Idle breathing, eye blinking, subtle head motion, physics, a tap reaction, /look and /mood provide presence. Speaking animation is driven by text activity, not claimed audio lip sync. The character never claims to be a real person, hides no tool activity, and does not claim correctness before verification. Her body language follows actual run state.

## Scope

Sessions are local and private. Resume, rename, fork, export and delete are explicit commands; forks share the project filesystem and do not roll back files. File tools are bounded to the selected project. Reads omit credential/private directories. Writes and shell commands require approval unless the user explicitly selected allow mode. Plan mode prohibits both. Checks are independent assertions against saved files. No automatic API retries. Budgets limit turns, tools, output tokens and time.

Model assets, SDK binaries, API keys, sessions and Chromium profiles are local dependencies, not repository contents. The older Python prototype stays available as historical source, but the `aster` entry point uses the Rust binary.
