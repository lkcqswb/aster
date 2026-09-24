//! Search the visible conversation without exposing private provider blocks.
use crate::session::{Entry, Session};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub enum Action {
    Keep,
    Close,
    Jump(usize),
}
pub struct History {
    pub items: Vec<Entry>,
    searchable: Vec<String>,
    pub query: String,
    pub matches: Vec<usize>,
    pub index: usize,
    pub preview: bool,
    pub scroll: u16,
    pub focus_match: bool,
    source: String,
}
impl History {
    pub fn new(session: &Session, query: String) -> Self {
        let mut history = Self {
            items: vec![],
            searchable: vec![],
            query,
            matches: vec![],
            index: 0,
            preview: false,
            scroll: 0,
            focus_match: false,
            source: session.id.clone(),
        };
        history.refresh(session);
        history
    }
    pub fn refresh(&mut self, session: &Session) {
        if self.source != session.id || self.items.len() > session.entries.len() {
            self.items.clear();
            self.searchable.clear();
            self.preview = false;
            self.index = 0;
            self.source = session.id.clone();
        }
        if self.items.len() != session.entries.len() {
            for entry in session.entries.iter().skip(self.items.len()) {
                self.searchable.push(entry.text.to_lowercase());
                self.items.push(entry.clone());
            }
            self.filter();
        }
    }
    fn filter(&mut self) {
        let query = self.query.to_lowercase();
        let (role, query) = match query.split_once(':') {
            Some((role @ ("you" | "nongyu" | "tool" | "notice"), rest)) => {
                (Some(role), rest.trim())
            }
            _ => (None, query.as_str()),
        };
        self.matches = self
            .items
            .iter()
            .enumerate()
            .filter(|(i, e)| {
                role.is_none_or(|r| e.role == r) && self.searchable[*i].contains(query)
            })
            .map(|(i, _)| i)
            .collect();
        self.index = self.index.min(self.matches.len().saturating_sub(1));
    }
    pub fn selected(&self) -> Option<usize> {
        self.matches.get(self.index).copied()
    }
    pub fn needle(&self) -> String {
        let query = self.query.to_lowercase();
        match query.split_once(':') {
            Some(("you" | "nongyu" | "tool" | "notice", rest)) => rest.trim().into(),
            _ => query,
        }
    }
    pub fn paste(&mut self, text: &str) {
        if !self.preview && self.query.len() + text.len() <= 160 {
            self.query.push_str(&text.replace('\n', " "));
            self.index = 0;
            self.filter();
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Action {
        if self.preview {
            match key.code {
                KeyCode::Esc => {
                    self.preview = false;
                    self.scroll = 0;
                }
                KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
                KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
                KeyCode::Home => {
                    self.focus_match = false;
                    self.scroll = 0;
                }
                KeyCode::End => {
                    self.focus_match = false;
                    self.scroll = u16::MAX;
                }
                KeyCode::Tab => {
                    if let Some(entry) = self.selected() {
                        return Action::Jump(entry);
                    }
                }
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Esc => return Action::Close,
                KeyCode::Enter => {
                    if self.selected().is_some() {
                        self.preview = true;
                        self.scroll = 0;
                        self.focus_match = true;
                    }
                }
                KeyCode::Tab => {
                    if let Some(entry) = self.selected() {
                        return Action::Jump(entry);
                    }
                }
                KeyCode::Up => self.index = self.index.saturating_sub(1),
                KeyCode::Down => {
                    self.index = (self.index + 1).min(self.matches.len().saturating_sub(1))
                }
                KeyCode::PageUp => self.index = self.index.saturating_sub(10),
                KeyCode::PageDown => {
                    self.index = (self.index + 10).min(self.matches.len().saturating_sub(1))
                }
                KeyCode::Home => self.index = 0,
                KeyCode::End => self.index = self.matches.len().saturating_sub(1),
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.query.clear();
                    self.index = 0;
                    self.filter();
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL) && self.query.len() < 160 =>
                {
                    self.query.push(c);
                    self.index = 0;
                    self.filter();
                }
                KeyCode::Backspace => {
                    self.query.pop();
                    self.index = 0;
                    self.filter();
                }
                _ => {}
            }
        }
        Action::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_filters_roles_unicode_and_appended_entries_without_provider_thinking() {
        let mut session = Session::new("/project".into(), "test".into(), true);
        session.add("you", "Keep 中文 and JADE");
        session.add("nongyu", "I can inspect jade");
        session.messages.push(serde_json::json!({"role":"assistant","content":[{"type":"thinking","thinking":"private jade"}]}));
        let mut history = History::new(&session, "you: jade".into());
        assert_eq!(history.matches, [0]);
        session.add("you", "More Jade");
        history.refresh(&session);
        assert_eq!(history.matches, [0, 2]);
        history.query = "中文".into();
        history.filter();
        assert_eq!(history.matches, [0]);
        history.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(history.preview);
        assert!(matches!(
            history.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            Action::Jump(0)
        ));
        history.query = "private".into();
        history.filter();
        assert!(history.matches.is_empty());
    }
}
