//! Text editing for the composer and small popup inputs.
//!
//! Cursor positions are byte offsets on character boundaries. Visual rows use the
//! same character-width wrapping as the rendered composer, so ↑/↓ move where the
//! eye expects them to.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Editor {
    pub text: String,
    pub cursor: usize,
}

#[derive(Debug, PartialEq)]
pub enum Edit {
    /// The key changed the text or moved the cursor.
    Handled,
    /// The cursor is on the first visual row and ↑ was pressed.
    AboveTop,
    /// The cursor is on the last visual row and ↓ was pressed.
    BelowBottom,
    /// Not an editing key.
    Ignored,
}

fn width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}
fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Editor {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            cursor: text.len(),
            text,
        }
    }
    pub fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
    }
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn insert(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
    }
    fn previous(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
    fn next(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map(|c| self.cursor + c.len_utf8())
            .unwrap_or(self.cursor)
    }
    fn word_left(&self) -> usize {
        let mut at = self.cursor;
        let before = &self.text[..at];
        let mut chars = before.char_indices().rev().peekable();
        while let Some(&(i, c)) = chars.peek() {
            if word(c) {
                break;
            }
            at = i;
            chars.next();
        }
        while let Some(&(i, c)) = chars.peek() {
            if !word(c) {
                break;
            }
            at = i;
            chars.next();
        }
        at
    }
    fn word_right(&self) -> usize {
        let mut at = self.cursor;
        let mut chars = self.text[at..].chars().peekable();
        while let Some(&c) = chars.peek() {
            if word(c) {
                break;
            }
            at += c.len_utf8();
            chars.next();
        }
        while let Some(&c) = chars.peek() {
            if !word(c) {
                break;
            }
            at += c.len_utf8();
            chars.next();
        }
        at
    }
    fn line_start(&self) -> usize {
        self.text[..self.cursor]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0)
    }
    fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map(|i| self.cursor + i)
            .unwrap_or(self.text.len())
    }
    /// Byte ranges of each wrapped row. A logical newline always starts a new row.
    pub fn rows(&self, columns: usize) -> Vec<(usize, usize)> {
        rows(&self.text, columns)
    }
    /// The visual row and display column of the cursor.
    pub fn cursor_position(&self, columns: usize) -> (usize, usize) {
        let rows = self.rows(columns);
        let row = cursor_row(&rows, &self.text, self.cursor);
        let (start, _) = rows[row];
        (row, self.text[start..self.cursor].chars().map(width).sum())
    }
    fn vertical(&mut self, columns: usize, down: bool) -> bool {
        let rows = self.rows(columns);
        let (row, column) = self.cursor_position(columns);
        let target = if down {
            if row + 1 >= rows.len() {
                return false;
            }
            row + 1
        } else {
            if row == 0 {
                return false;
            }
            row - 1
        };
        let (start, end) = rows[target];
        // At a soft wrap the row's end is the next row's start; stay on this row.
        let soft_wrap = rows.get(target + 1).is_some_and(|next| next.0 == end);
        let mut used = 0;
        let mut at = start;
        for (i, c) in self.text[start..end].char_indices() {
            let after = start + i + c.len_utf8();
            if used + width(c) > column || (soft_wrap && after == end) {
                break;
            }
            used += width(c);
            at = after;
        }
        self.cursor = at;
        true
    }
    /// Apply one key. `columns` is the width used to wrap the text on screen.
    pub fn key(&mut self, key: KeyEvent, columns: usize) -> Edit {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Left if control || alt => self.cursor = self.word_left(),
            KeyCode::Right if control || alt => self.cursor = self.word_right(),
            KeyCode::Char('b') if alt => self.cursor = self.word_left(),
            KeyCode::Char('f') if alt => self.cursor = self.word_right(),
            KeyCode::Left => self.cursor = self.previous(),
            KeyCode::Right => self.cursor = self.next(),
            KeyCode::Char('b') if control => self.cursor = self.previous(),
            KeyCode::Char('f') if control => self.cursor = self.next(),
            // A second Home/End reaches the start/end of the whole draft.
            KeyCode::Home if self.cursor == self.line_start() => self.cursor = 0,
            KeyCode::End if self.cursor == self.line_end() => self.cursor = self.text.len(),
            KeyCode::Home => self.cursor = self.line_start(),
            KeyCode::End => self.cursor = self.line_end(),
            KeyCode::Char('a') if control => self.cursor = self.line_start(),
            KeyCode::Char('e') if control => self.cursor = self.line_end(),
            KeyCode::Up => {
                if !self.vertical(columns, false) {
                    return Edit::AboveTop;
                }
            }
            KeyCode::Down => {
                if !self.vertical(columns, true) {
                    return Edit::BelowBottom;
                }
            }
            KeyCode::Backspace if alt || control => {
                let at = self.word_left();
                self.text.replace_range(at..self.cursor, "");
                self.cursor = at;
            }
            KeyCode::Char('w') if control => {
                let at = self.word_left();
                self.text.replace_range(at..self.cursor, "");
                self.cursor = at;
            }
            KeyCode::Char('h') if control => {
                let at = self.previous();
                self.text.replace_range(at..self.cursor, "");
                self.cursor = at;
            }
            KeyCode::Backspace => {
                let at = self.previous();
                self.text.replace_range(at..self.cursor, "");
                self.cursor = at;
            }
            KeyCode::Delete if alt || control => {
                let end = self.word_right();
                self.text.replace_range(self.cursor..end, "");
            }
            KeyCode::Char('d') if alt => {
                let end = self.word_right();
                self.text.replace_range(self.cursor..end, "");
            }
            KeyCode::Delete => {
                let end = self.next();
                self.text.replace_range(self.cursor..end, "");
            }
            KeyCode::Char('d') if control => {
                let end = self.next();
                self.text.replace_range(self.cursor..end, "");
            }
            KeyCode::Char('u') if control => {
                let start = self.line_start();
                let start = if start == self.cursor && start > 0 {
                    start - 1
                } else {
                    start
                };
                self.text.replace_range(start..self.cursor, "");
                self.cursor = start;
            }
            KeyCode::Char('k') if control => {
                let end = self.line_end();
                let end = if end == self.cursor && end < self.text.len() {
                    end + 1
                } else {
                    end
                };
                self.text.replace_range(self.cursor..end, "");
            }
            KeyCode::Char(c) if !control && !alt && !c.is_control() => {
                let mut buffer = [0; 4];
                self.insert(c.encode_utf8(&mut buffer));
            }
            _ => return Edit::Ignored,
        }
        Edit::Handled
    }
}

pub fn rows(text: &str, columns: usize) -> Vec<(usize, usize)> {
    let columns = columns.max(1);
    let mut rows = vec![];
    let mut start = 0;
    let mut used = 0;
    for (i, c) in text.char_indices() {
        if c == '\n' {
            rows.push((start, i));
            start = i + 1;
            used = 0;
            continue;
        }
        let w = width(c);
        if used + w > columns && i > start {
            rows.push((start, i));
            start = i;
            used = 0;
        }
        used += w;
    }
    rows.push((start, text.len()));
    rows
}

/// A cursor at a soft wrap boundary belongs to the next row, like typed text.
pub fn cursor_row(rows: &[(usize, usize)], text: &str, cursor: usize) -> usize {
    for (index, &(start, end)) in rows.iter().enumerate() {
        if cursor < end || (cursor == end && start <= cursor) {
            let soft_wrap = cursor == end
                && index + 1 < rows.len()
                && rows[index + 1].0 == end
                && !text[..end].ends_with('\n');
            if soft_wrap {
                return index + 1;
            }
            if cursor >= start {
                return index;
            }
        }
    }
    rows.len() - 1
}

/// Browsing earlier prompts with ↑/↓ without losing the unsent draft.
#[derive(Default)]
pub struct PromptHistory {
    items: Vec<String>,
    position: Option<usize>,
    draft: Option<String>,
}
impl PromptHistory {
    pub fn with(items: Vec<String>) -> Self {
        let mut history = Self::default();
        for item in items {
            history.push(&item);
        }
        history
    }
    pub fn push(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() || text.len() > 32_000 {
            return;
        }
        self.items.retain(|item| item != text);
        self.items.push(text.into());
        if self.items.len() > 200 {
            self.items.remove(0);
        }
        self.reset();
    }
    pub fn reset(&mut self) {
        self.position = None;
        self.draft = None;
    }
    pub fn browsing(&self) -> bool {
        self.position.is_some()
    }
    pub fn older(&mut self, current: &str) -> Option<String> {
        let next = match self.position {
            None if self.items.is_empty() => return None,
            None => {
                self.draft = Some(current.into());
                self.items.len() - 1
            }
            Some(0) => return None,
            Some(i) => i - 1,
        };
        self.position = Some(next);
        Some(self.items[next].clone())
    }
    pub fn newer(&mut self) -> Option<String> {
        let i = self.position?;
        if i + 1 < self.items.len() {
            self.position = Some(i + 1);
            Some(self.items[i + 1].clone())
        } else {
            self.position = None;
            Some(self.draft.take().unwrap_or_default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn with(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }
    #[test]
    fn arrows_move_between_lines_before_leaving_the_draft() {
        let mut e = Editor::new("first line\nsecond");
        assert_eq!(e.cursor_position(40), (1, 6));
        assert_eq!(e.key(key(KeyCode::Up), 40), Edit::Handled);
        assert_eq!(e.cursor_position(40), (0, 6));
        assert_eq!(&e.text[..e.cursor], "first ");
        assert_eq!(e.key(key(KeyCode::Up), 40), Edit::AboveTop);
        assert_eq!(e.key(key(KeyCode::Down), 40), Edit::Handled);
        assert_eq!(e.key(key(KeyCode::Down), 40), Edit::BelowBottom);
    }
    #[test]
    fn soft_wrapped_rows_and_wide_characters_keep_their_columns() {
        let mut e = Editor::new("弄玉弄玉弄玉");
        // Four columns hold two wide characters per row.
        assert_eq!(e.rows(4).len(), 3);
        assert_eq!(e.cursor_position(4), (2, 4));
        e.key(key(KeyCode::Up), 4);
        assert_eq!(e.cursor_position(4), (1, 2));
        e.key(key(KeyCode::Up), 4);
        assert_eq!(e.cursor_position(4), (0, 2));
        assert_eq!(e.key(key(KeyCode::Up), 4), Edit::AboveTop);
        e.cursor = "弄玉".len();
        // The boundary belongs to the next row, where typing continues.
        assert_eq!(e.cursor_position(4), (1, 0));
    }
    #[test]
    fn word_motion_and_deletion_follow_readline() {
        let mut e = Editor::new("cargo test --release");
        e.key(with(KeyCode::Left, KeyModifiers::ALT), 80);
        assert_eq!(&e.text[e.cursor..], "release");
        e.key(with(KeyCode::Char('b'), KeyModifiers::ALT), 80);
        assert_eq!(&e.text[e.cursor..], "test --release");
        e.key(with(KeyCode::Char('f'), KeyModifiers::ALT), 80);
        assert_eq!(&e.text[e.cursor..], " --release");
        e.key(with(KeyCode::Char('w'), KeyModifiers::CONTROL), 80);
        assert_eq!(e.text, "cargo  --release");
        e.key(with(KeyCode::Char('k'), KeyModifiers::CONTROL), 80);
        assert_eq!(e.text, "cargo ");
        e.key(with(KeyCode::Char('u'), KeyModifiers::CONTROL), 80);
        assert!(e.is_empty());
        // Option+arrow must never type letters into the draft.
        let mut e = Editor::new("abc");
        e.key(with(KeyCode::Char('b'), KeyModifiers::ALT), 80);
        assert_eq!(e.text, "abc");
        assert_eq!(e.cursor, 0);
    }
    #[test]
    fn home_end_and_kill_work_on_the_current_line() {
        let mut e = Editor::new("one\ntwo three");
        e.key(key(KeyCode::Home), 80);
        assert_eq!(&e.text[e.cursor..], "two three");
        e.key(key(KeyCode::Home), 80);
        assert_eq!(e.cursor, 0);
        e.key(key(KeyCode::End), 80);
        assert_eq!(e.cursor, 3);
        e.key(key(KeyCode::End), 80);
        assert_eq!(e.cursor, e.text.len());
        e.key(with(KeyCode::Char('u'), KeyModifiers::CONTROL), 80);
        assert_eq!(e.text, "one\n");
        e.key(with(KeyCode::Char('u'), KeyModifiers::CONTROL), 80);
        assert_eq!(e.text, "one");
    }
    #[test]
    fn history_recall_keeps_the_unsent_draft() {
        let mut h = PromptHistory::with(vec!["first".into(), "second".into(), "first".into()]);
        assert_eq!(h.older("draft").as_deref(), Some("first"));
        assert_eq!(h.older("first").as_deref(), Some("second"));
        assert_eq!(h.older("second"), None);
        assert_eq!(h.newer().as_deref(), Some("first"));
        assert_eq!(h.newer().as_deref(), Some("draft"));
        assert!(!h.browsing());
        assert_eq!(h.newer(), None);
    }
}
