# Reading earlier conversations with 弄玉

Press **F7**, choose history in the **F1** companion menu, or use `/history [text]`. Search covers the visible entries in the current conversation, including the transcript retained after context compaction. It does not search private provider thinking or other sessions.

- Type or paste to filter, case-insensitively. `you:`, `nongyu:`, `tool:` and `notice:` restrict the role, for example `/history you: output format`.
- Up/Down select; Page Up/Down move ten results; Home/End select the first/last match.
- Enter reads the complete entry. The preview starts near a matching displayed line when possible. Up/Down and Page Up/Down scroll; Home/End read the beginning/end; Escape returns to results.
- Tab locates the selected entry in the conversation. **Ctrl+End** returns to the latest messages. Neither action submits or changes your draft.
- While an approval or question is pending, the preview is available but jumping out to the transcript is held until the decision is resolved. Escape returns to the original decision with its answer draft intact.

弄玉 uses her reading animation while you inspect an entry. The search refreshes when entries are appended. Idle redraws reuse the transcript layout; new replies keep a scrolled view in place. Resizing and changing tool detail invalidate the cached layout. The cache is local and contains only the already-visible transcript.
