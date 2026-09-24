//! Local project browsing. Reading runs off the terminal thread and never calls a model.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Files,
    Search,
}
#[derive(Clone, Debug)]
struct Hit {
    path: String,
    line: Option<usize>,
    excerpt: String,
}
struct Job {
    receiver: crossbeam_channel::Receiver<Result<Value, String>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
struct Preview {
    hit: Hit,
    offset: usize,
    column: usize,
    text: String,
    lines: usize,
    next: Option<(usize, usize)>,
    history: Vec<(usize, usize)>,
    scroll: u16,
}
pub enum Action {
    Keep,
    Close,
    Attach(String),
    Notice(String),
}
pub struct Navigator {
    root: PathBuf,
    mode: Mode,
    query: String,
    hits: Vec<Hit>,
    index: usize,
    offset: usize,
    next: Option<usize>,
    history: Vec<usize>,
    preview: Option<Preview>,
    job: Option<Job>,
    dirty: Option<Instant>,
    error: String,
    detail: String,
}
impl Navigator {
    pub fn new(root: PathBuf, mode: Mode, query: String) -> Self {
        let mut nav = Self {
            root,
            mode,
            query,
            hits: vec![],
            index: 0,
            offset: 0,
            next: None,
            history: vec![],
            preview: None,
            job: None,
            dirty: None,
            error: String::new(),
            detail: String::new(),
        };
        nav.load_results();
        nav
    }
    fn job(&mut self, tool: &'static str, args: Value) {
        self.job = None;
        let root = self.root.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancelled = cancel.clone();
        let (sender, receiver) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let result =
                crate::tools::execute(&root, tool, &args, &cancelled).map_err(|e| e.to_string());
            let _ = sender.send(result);
        });
        self.error.clear();
        self.job = Some(Job { receiver, cancel });
    }
    fn load_results(&mut self) {
        self.preview = None;
        self.hits.clear();
        self.index = 0;
        self.next = None;
        if self.mode == Mode::Search && self.query.trim().is_empty() {
            self.detail = "Type the text you want to find.".into();
            return;
        }
        let mut args = json!({"offset":self.offset,"limit":80,"case_sensitive":false});
        if self.mode == Mode::Files {
            if !self.query.is_empty() {
                let pattern = if self.query.contains(['*', '?', '[', '{']) {
                    self.query.clone()
                } else {
                    format!("**/*{}*", globset::escape(&self.query))
                };
                args["glob"] = json!(pattern);
            }
            self.job("list_files", args);
        } else {
            args["query"] = json!(self.query);
            args["case_sensitive"] = json!(false);
            self.job("search", args);
        }
    }
    fn load_preview(&mut self) {
        if let Some(preview) = &mut self.preview {
            preview.text.clear();
            preview.scroll = 0;
            preview.next = None;
            let args = json!({"path":preview.hit.path,"offset":preview.offset,"column":preview.column,"limit":40});
            self.job("read_file", args);
        }
    }
    pub fn tick(&mut self) {
        if self
            .dirty
            .is_some_and(|at| at.elapsed() >= Duration::from_millis(180))
        {
            self.dirty = None;
            self.load_results();
        }
        let result = self
            .job
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    Some(Err("Project reader stopped unexpectedly".into()))
                }
                Err(_) => None,
            });
        let Some(result) = result else {
            return;
        };
        self.job = None;
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                self.error = error;
                return;
            }
        };
        if let Some(preview) = &mut self.preview {
            preview.text = value["content"].as_str().unwrap_or("").into();
            preview.lines = value["returned_lines"].as_u64().unwrap_or(0) as usize;
            preview.next = value["next_offset"].as_u64().map(|line| {
                (
                    line as usize,
                    value["next_column"].as_u64().unwrap_or(1) as usize,
                )
            });
            self.detail = format!(
                "{} lines · {} KB source",
                value["total_lines"],
                value["source_bytes"].as_u64().unwrap_or(0) / 1000
            );
        } else {
            self.next = value["next_offset"].as_u64().map(|offset| offset as usize);
            self.hits = if self.mode == Mode::Files {
                value["files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|path| Hit {
                        path: path.into(),
                        line: None,
                        excerpt: String::new(),
                    })
                    .collect()
            } else {
                value["matches"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|hit| Hit {
                        path: hit["path"].as_str().unwrap_or("").into(),
                        line: hit["line"].as_u64().map(|n| n as usize),
                        excerpt: hit["text"].as_str().unwrap_or("").into(),
                    })
                    .collect()
            };
            self.detail = format!(
                "{} {} · {}",
                self.hits.len(),
                if self.hits.len() == 1 {
                    "result"
                } else {
                    "results"
                },
                if self.next.is_some() {
                    "more on the next page"
                } else {
                    "end of results"
                }
            );
            if let Some(reason) = value["incomplete_reason"].as_str() {
                self.detail = reason.into();
            }
            let skipped = value["skipped_files"].as_u64().unwrap_or(0);
            let errors = value["scan_errors"].as_u64().unwrap_or(0);
            if skipped + errors > 0 {
                self.detail += &format!(" · {skipped} skipped · {errors} scan errors");
            }
        }
    }
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.dirty.is_some()
    }
    pub fn reading(&self) -> bool {
        self.preview.is_some()
    }
    fn changed_query(&mut self) {
        self.job = None;
        self.dirty = Some(Instant::now());
        self.offset = 0;
        self.history.clear();
        self.index = 0;
        self.error.clear();
    }
    pub fn paste(&mut self, text: &str) {
        if self.preview.is_none() && self.query.len() + text.len() <= 500 {
            self.query.push_str(&text.replace(['\r', '\n'], " "));
            self.changed_query();
        }
    }
    fn reference(&self) -> Result<String, String> {
        let (hit, first, last) = if let Some(preview) = &self.preview {
            if preview.column > 1 {
                return Err(
                    "This is part of a long line. Ask 弄玉 to continue it with read_file.".into(),
                );
            }
            (
                &preview.hit,
                preview.offset,
                preview.offset + preview.lines.saturating_sub(1),
            )
        } else {
            let hit = self.hits.get(self.index).ok_or("Choose a file first")?;
            let first = hit
                .line
                .map(|line| line.saturating_sub(2).max(1))
                .unwrap_or(1);
            let last = hit.line.map(|line| line + 2).unwrap_or(80);
            (hit, first, last)
        };
        if hit.path.contains(['}', '\n', '\r']) {
            return Err("This filename cannot be represented as an @ reference.".into());
        }
        let reference = format!("@{{{}:{first}-{last}}}", hit.path);
        crate::context::prepare(&self.root, &reference, &crate::context::Catalog::default())
            .map_err(|e| e.to_string())?;
        Ok(reference)
    }
    pub fn key(&mut self, key: KeyEvent) -> Action {
        if key.code == KeyCode::Esc {
            self.job = None;
            if self.preview.take().is_some() {
                self.error.clear();
                self.detail = format!(
                    "{} {} · {}",
                    self.hits.len(),
                    if self.hits.len() == 1 {
                        "result"
                    } else {
                        "results"
                    },
                    if self.next.is_some() {
                        "more on the next page"
                    } else {
                        "end of results"
                    }
                );
                return Action::Keep;
            }
            return Action::Close;
        }
        if key.code == KeyCode::Tab && !self.busy() && self.error.is_empty() {
            return match self.reference() {
                Ok(reference) => Action::Attach(reference),
                Err(error) => Action::Notice(error),
            };
        }
        if let Some(preview) = &mut self.preview {
            match key.code {
                KeyCode::Down => preview.scroll = preview.scroll.saturating_add(1),
                KeyCode::Up => preview.scroll = preview.scroll.saturating_sub(1),
                KeyCode::PageDown => preview.scroll = preview.scroll.saturating_add(10),
                KeyCode::PageUp => preview.scroll = preview.scroll.saturating_sub(10),
                KeyCode::Right if self.job.is_none() => {
                    if let Some((offset, column)) = preview.next {
                        preview.history.push((preview.offset, preview.column));
                        preview.offset = offset;
                        preview.column = column;
                        self.load_preview();
                    }
                }
                KeyCode::Left if self.job.is_none() => {
                    if let Some((offset, column)) = preview.history.pop() {
                        preview.offset = offset;
                        preview.column = column;
                        self.load_preview();
                    }
                }
                _ => {}
            }
            return Action::Keep;
        }
        match key.code {
            KeyCode::Down => self.index = (self.index + 1).min(self.hits.len().saturating_sub(1)),
            KeyCode::Up => self.index = self.index.saturating_sub(1),
            KeyCode::Enter if !self.busy() => {
                if let Some(hit) = self.hits.get(self.index).cloned() {
                    self.preview = Some(Preview {
                        offset: hit
                            .line
                            .map(|line| line.saturating_sub(4).max(1))
                            .unwrap_or(1),
                        column: 1,
                        hit,
                        text: String::new(),
                        lines: 0,
                        next: None,
                        history: vec![],
                        scroll: 0,
                    });
                    self.load_preview();
                }
            }
            KeyCode::PageDown if !self.busy() => {
                if let Some(offset) = self.next {
                    self.history.push(self.offset);
                    self.offset = offset;
                    self.load_results();
                }
            }
            KeyCode::PageUp if !self.busy() => {
                if let Some(offset) = self.history.pop() {
                    self.offset = offset;
                    self.load_results();
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.clear();
                self.changed_query();
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.changed_query();
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && self.query.len() + c.len_utf8() <= 500 =>
            {
                self.query.push(c);
                self.changed_query();
            }
            _ => {}
        }
        Action::Keep
    }
    pub fn view(&self, width: usize, height: usize) -> (String, String, u16) {
        let status = if self.busy() {
            "Reading the project…"
        } else if !self.error.is_empty() {
            &self.error
        } else {
            &self.detail
        };
        if let Some(preview) = &self.preview {
            return (
                format!("Reading · {}", preview.hit.path),
                format!(
                    "{status}\nTab attach these lines · ←→ pages · Esc back\n\n{}",
                    preview.text
                ),
                preview.scroll,
            );
        }
        let title = if self.mode == Mode::Files {
            "Files beside 弄玉"
        } else {
            "Search beside 弄玉"
        };
        let mut text = format!(
            "{}: {}\n{}\n\n",
            if self.mode == Mode::Files {
                "Name or glob"
            } else {
                "Find text"
            },
            fit(&self.query, width.saturating_sub(14)),
            fit(status, width)
        );
        let rows = if self.mode == Mode::Files { 1 } else { 2 };
        let visible = height
            .saturating_sub(9)
            .checked_div(rows)
            .unwrap_or(1)
            .max(1);
        for (index, hit) in self
            .hits
            .iter()
            .enumerate()
            .skip(self.index.saturating_sub(visible - 1))
            .take(visible)
        {
            let location = hit
                .line
                .map(|line| format!("{}:{line}", hit.path))
                .unwrap_or_else(|| hit.path.clone());
            text += &format!(
                "{} {}\n",
                if index == self.index { "›" } else { " " },
                fit(&location, width.saturating_sub(2))
            );
            if rows == 2 {
                text += &format!("  {}\n", fit(hit.excerpt.trim(), width.saturating_sub(2)));
            }
        }
        if self.hits.is_empty() && !self.busy() {
            text += "No matching results.\n";
        }
        text += "\n↑↓ choose · Enter read · Tab attach\nPgUp/PgDn pages · Ctrl+U clear · Esc close";
        (title.into(), text, 0)
    }
}
fn fit(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars().filter(|c| !c.is_control()) {
        let size = c.width().unwrap_or(0);
        if used + size > width.saturating_sub(1) {
            out.push('…');
            return out;
        }
        out.push(c);
        used += size;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn finish(nav: &mut Navigator) {
        let deadline = Instant::now() + Duration::from_secs(4);
        while nav.busy() {
            assert!(Instant::now() < deadline);
            nav.tick();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(nav.error.is_empty(), "{}", nav.error);
    }
    #[test]
    fn replacing_a_search_cancels_old_results_and_respects_filename_case() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("alpha.txt"), "alpha").unwrap();
        fs::write(dir.path().join("beta.txt"), "beta").unwrap();
        let mut nav = Navigator::new(dir.path().into(), Mode::Files, "alpha".into());
        nav.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        nav.paste("BETA");
        finish(&mut nav);
        assert_eq!(nav.hits.len(), 1);
        assert_eq!(nav.hits[0].path, "beta.txt");
        assert!(
            matches!(nav.key(key(KeyCode::Tab)), Action::Attach(reference) if reference=="@{beta.txt:1-80}")
        );
    }
    #[test]
    fn large_file_preview_and_attachment_keep_the_exact_selected_range() {
        let dir = tempfile::tempdir().unwrap();
        let source = format!(
            "{}needle jade\nlast\n",
            "unchanged source line\n".repeat(8000)
        );
        fs::write(dir.path().join("big source.txt"), &source).unwrap();
        let mut nav = Navigator::new(dir.path().into(), Mode::Search, "NEEDLE".into());
        finish(&mut nav);
        assert_eq!(nav.hits[0].line, Some(8001));
        nav.key(key(KeyCode::Enter));
        finish(&mut nav);
        assert!(nav.view(72, 24).1.contains("8001: needle jade"));
        let Action::Attach(reference) = nav.key(key(KeyCode::Tab)) else {
            panic!("No attachment")
        };
        assert_eq!(reference, "@{big source.txt:7997-8002}");
        let prepared =
            crate::context::prepare(dir.path(), &reference, &crate::context::Catalog::default())
                .unwrap();
        assert!(prepared.content.contains("8001: needle jade"));
        assert!(!prepared.content.contains("1: unchanged"));
        assert!(matches!(nav.key(key(KeyCode::Esc)), Action::Keep));
        assert!(nav.preview.is_none());
        assert!(matches!(nav.key(key(KeyCode::Esc)), Action::Close));
        assert_eq!(
            fs::read_to_string(dir.path().join("big source.txt")).unwrap(),
            source
        );
    }
    #[test]
    fn result_and_source_pages_can_be_traversed_backwards() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..81 {
            fs::write(
                dir.path().join(format!("file-{i:03}.txt")),
                "line\n".repeat(100),
            )
            .unwrap();
        }
        let mut nav = Navigator::new(dir.path().into(), Mode::Files, String::new());
        finish(&mut nav);
        assert_eq!(nav.hits.len(), 80);
        nav.key(key(KeyCode::PageDown));
        finish(&mut nav);
        assert_eq!(nav.hits[0].path, "file-080.txt");
        nav.key(key(KeyCode::PageUp));
        finish(&mut nav);
        assert_eq!(nav.hits[0].path, "file-000.txt");
        nav.key(key(KeyCode::Enter));
        finish(&mut nav);
        nav.key(key(KeyCode::Right));
        finish(&mut nav);
        assert_eq!(nav.preview.as_ref().unwrap().offset, 41);
        nav.key(key(KeyCode::Left));
        finish(&mut nav);
        assert_eq!(nav.preview.as_ref().unwrap().offset, 1);
    }
}
