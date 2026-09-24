# Aster development

Aster's active application is the Rust crate in `src/`. The earlier `relay/` Python prototype is retained for reference, not used by the launcher.

- Keep the terminal conversational and restrained. 弄玉's actual Live2D animation is central to the experience.
- Keep model assets and vendor SDKs local. Do not copy or commit assets from `~/desktop-pet`.
- Never print or commit `.env`, credentials, session stores, or private browser profiles.
- Preserve terminal restoration and renderer/process cleanup on exit.
- Do not confuse a model's final message with an independent check passing.
- File mutations and shell commands obey permission settings; plan mode remains read-only.
- Session forks share project files; never imply filesystem rollback.
- Preserve full provider content blocks privately; display only user-visible text and tool activity.

Validation: `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo build --release`. For terminal behavior, run the demo PTY test in `scripts/test_rust_e2e.py`. Live API calls must remain explicit and bounded; do not retry failures automatically.
