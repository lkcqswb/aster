//! The models and API keys panel: choose a model, add providers, keys and model names.
//!
//! Nothing here makes a request except an explicit test (`t`), which the app runs.
use crate::{
    composer::Editor,
    providers::{Auth, Model, PRESETS, Provider, Registry, masked},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// MiniMax configured through `.env` or the environment.
    Env,
    Model {
        provider: usize,
        model: usize,
    },
    /// A provider that has no model names yet.
    Empty {
        provider: usize,
    },
    Demo,
    Add,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Keep,
    Close,
    /// Use this model for the open conversation and new ones.
    Use {
        provider: Option<String>,
        model: String,
        demo: bool,
    },
    /// Run one tiny request against this provider and model.
    Test {
        provider: String,
        model: String,
    },
    Saved(String),
}

const FIELDS: [&str; 6] = [
    "Preset",
    "Name",
    "Base URL",
    "Key header",
    "API key",
    "Models",
];
const PRESET: usize = 0;
const NAME: usize = 1;
const BASE: usize = 2;
const AUTH: usize = 3;
const KEY: usize = 4;
const MODELS: usize = 5;

pub struct Form {
    pub replacing: Option<String>,
    pub preset: usize,
    pub auth: Auth,
    /// Name, base, key and models, indexed by field.
    pub text: [Editor; 6],
    pub focus: usize,
    /// The saved key's mask, shown while the key field is left empty.
    pub existing_key: Option<String>,
    pub error: String,
}
impl Form {
    fn new(preset: usize) -> Self {
        let mut form = Self {
            replacing: None,
            preset,
            auth: Auth::Bearer,
            text: Default::default(),
            focus: NAME,
            existing_key: None,
            error: String::new(),
        };
        form.apply_preset();
        form
    }
    fn edit(provider: &Provider, focus: usize) -> Self {
        let mut form = Self::new(PRESETS.len() - 1);
        form.replacing = Some(provider.id.clone());
        form.text[NAME].set(provider.name.clone());
        form.text[BASE].set(provider.base.clone());
        form.auth = provider.auth;
        form.text[MODELS].set(models_text(&provider.models));
        form.existing_key = Some(masked(&provider.key));
        form.focus = focus;
        form
    }
    fn apply_preset(&mut self) {
        let preset = &PRESETS[self.preset];
        self.text[NAME].set(if self.preset + 1 == PRESETS.len() {
            ""
        } else {
            preset.name
        });
        self.text[BASE].set(preset.base);
        self.auth = preset.auth;
        let models = preset
            .models
            .iter()
            .map(|(name, context)| Model {
                name: (*name).into(),
                context: *context,
            })
            .collect::<Vec<_>>();
        self.text[MODELS].set(models_text(&models));
    }
    fn provider(&self) -> Result<Provider, String> {
        let mut models = vec![];
        for item in self.text[MODELS]
            .text
            .split([',', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let (name, context) = match item.split_once('=') {
                Some((name, context)) => (
                    name.trim(),
                    Some(
                        context
                            .trim()
                            .replace('_', "")
                            .parse::<u64>()
                            .map_err(|_| format!("{item}: use name=tokens, e.g. {name}=200000"))?,
                    ),
                ),
                None => (item, None),
            };
            models.push(Model {
                name: name.into(),
                context,
            });
        }
        Ok(Provider {
            id: String::new(),
            name: self.text[NAME].text.clone(),
            base: self.text[BASE].text.clone(),
            auth: self.auth,
            key: self.text[KEY].text.clone(),
            models,
        })
    }
}
fn models_text(models: &[Model]) -> String {
    models
        .iter()
        .map(|m| match m.context {
            Some(c) => format!("{}={c}", m.name),
            None => m.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub enum View {
    List,
    Form(Box<Form>),
    Confirm(String),
}

pub struct Panel {
    pub path: PathBuf,
    pub registry: Registry,
    pub index: usize,
    pub view: View,
    pub current: (Option<String>, String, bool),
    pub env_key: bool,
    pub env_model: String,
    pub message: String,
}

impl Panel {
    pub fn open(
        path: PathBuf,
        current: (Option<String>, String, bool),
        env_key: bool,
        env_model: String,
    ) -> Self {
        let (registry, message) = match Registry::load(&path) {
            Ok(r) => (r, String::new()),
            Err(e) => (Registry::default(), format!("{e:#}")),
        };
        let mut panel = Self {
            path,
            registry,
            index: 0,
            view: View::List,
            current,
            env_key,
            env_model,
            message,
        };
        let rows = panel.rows();
        panel.index = rows
            .iter()
            .position(|row| panel.is_current(row))
            .unwrap_or(0);
        panel
    }
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Env];
        for (p, provider) in self.registry.providers.iter().enumerate() {
            if provider.models.is_empty() {
                rows.push(Row::Empty { provider: p });
            }
            for m in 0..provider.models.len() {
                rows.push(Row::Model {
                    provider: p,
                    model: m,
                });
            }
        }
        rows.push(Row::Demo);
        rows.push(Row::Add);
        rows
    }
    fn is_current(&self, row: &Row) -> bool {
        let (provider, model, demo) = &self.current;
        match row {
            Row::Demo => *demo,
            Row::Env => !demo && provider.is_none(),
            Row::Model {
                provider: p,
                model: m,
            } => {
                let entry = &self.registry.providers[*p];
                !demo
                    && provider.as_deref() == Some(entry.id.as_str())
                    && entry.models[*m].name == *model
            }
            _ => false,
        }
    }
    fn selected_provider(&self) -> Option<usize> {
        match self.rows().get(self.index) {
            Some(Row::Model { provider, .. } | Row::Empty { provider }) => Some(*provider),
            _ => None,
        }
    }
    fn save(&mut self) -> Result<(), String> {
        self.registry.save(&self.path).map_err(|e| format!("{e:#}"))
    }
    pub fn paste(&mut self, text: &str) {
        if let View::Form(form) = &mut self.view
            && form.focus != PRESET
            && form.focus != AUTH
        {
            let text = text.replace(['\r', '\n'], if form.focus == MODELS { ", " } else { "" });
            let editor = &mut form.text[form.focus];
            if editor.text.len() + text.len() <= 8192 {
                editor.insert(text.trim());
            }
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match &mut self.view {
            View::Confirm(id) => {
                if key.code == KeyCode::Char('y') {
                    let id = id.clone();
                    let name = self
                        .registry
                        .find(&id)
                        .map(|p| p.name.clone())
                        .unwrap_or_default();
                    self.registry.remove(&id);
                    self.view = View::List;
                    return match self.save() {
                        Ok(()) => {
                            self.index = self.index.min(self.rows().len() - 1);
                            Outcome::Saved(format!("Removed {name} and its key"))
                        }
                        Err(e) => {
                            self.message = e;
                            Outcome::Keep
                        }
                    };
                }
                self.view = View::List;
                Outcome::Keep
            }
            View::Form(form) => {
                let last = FIELDS.len() - 1;
                match key.code {
                    KeyCode::Esc => {
                        self.view = View::List;
                        return Outcome::Keep;
                    }
                    KeyCode::Tab | KeyCode::Down => form.focus = (form.focus + 1).min(last),
                    KeyCode::BackTab | KeyCode::Up => form.focus = form.focus.saturating_sub(1),
                    KeyCode::Char('s') if control => return self.submit(),
                    KeyCode::Enter if form.focus == last => return self.submit(),
                    KeyCode::Enter => form.focus += 1,
                    KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if form.focus == PRESET => {
                        let n = PRESETS.len();
                        form.preset = if key.code == KeyCode::Left {
                            (form.preset + n - 1) % n
                        } else {
                            (form.preset + 1) % n
                        };
                        if form.replacing.is_none() {
                            form.apply_preset();
                        }
                    }
                    KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if form.focus == AUTH => {
                        form.auth = if form.auth == Auth::Bearer {
                            Auth::XApiKey
                        } else {
                            Auth::Bearer
                        }
                    }
                    _ if form.focus != PRESET && form.focus != AUTH => {
                        form.text[form.focus].key(key, usize::MAX);
                        form.error.clear();
                    }
                    _ => {}
                }
                Outcome::Keep
            }
            View::List => {
                let rows = self.rows();
                let last = rows.len() - 1;
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => return Outcome::Close,
                    KeyCode::Up => {
                        self.index = if self.index == 0 {
                            last
                        } else {
                            self.index - 1
                        }
                    }
                    KeyCode::Down => {
                        self.index = if self.index >= last {
                            0
                        } else {
                            self.index + 1
                        }
                    }
                    KeyCode::PageUp => self.index = self.index.saturating_sub(5),
                    KeyCode::PageDown => self.index = (self.index + 5).min(last),
                    KeyCode::Home => self.index = 0,
                    KeyCode::End => self.index = last,
                    KeyCode::Char('a') => self.view = View::Form(Box::new(Form::new(0))),
                    KeyCode::Char('e') | KeyCode::Char('k') | KeyCode::Char('m') => {
                        if let Some(p) = self.selected_provider() {
                            let focus = match key.code {
                                KeyCode::Char('k') => KEY,
                                KeyCode::Char('m') => MODELS,
                                _ => NAME,
                            };
                            self.view = View::Form(Box::new(Form::edit(
                                &self.registry.providers[p],
                                focus,
                            )));
                        } else {
                            self.message =
                                "Choose one of your providers first · a adds a provider".into();
                        }
                    }
                    KeyCode::Char('d') | KeyCode::Delete => {
                        if let Some(p) = self.selected_provider() {
                            self.view = View::Confirm(self.registry.providers[p].id.clone());
                        } else {
                            self.message =
                                "Only providers you added can be removed here; .env stays as it is"
                                    .into();
                        }
                    }
                    KeyCode::Char('t') => {
                        if let Some(Row::Model { provider, model }) = rows.get(self.index) {
                            let p = &self.registry.providers[*provider];
                            return Outcome::Test {
                                provider: p.id.clone(),
                                model: p.models[*model].name.clone(),
                            };
                        }
                        self.message = "Choose a model row to test".into();
                    }
                    KeyCode::Enter => match rows.get(self.index) {
                        Some(Row::Add) => self.view = View::Form(Box::new(Form::new(0))),
                        Some(Row::Empty { provider }) => {
                            self.view = View::Form(Box::new(Form::edit(
                                &self.registry.providers[*provider],
                                MODELS,
                            )))
                        }
                        Some(Row::Demo) => {
                            return Outcome::Use {
                                provider: self.current.0.clone(),
                                model: self.current.1.clone(),
                                demo: true,
                            };
                        }
                        Some(Row::Env) => {
                            self.registry.default = None;
                            if let Err(e) = self.save() {
                                self.message = e;
                                return Outcome::Keep;
                            }
                            return Outcome::Use {
                                provider: None,
                                model: self.env_model.clone(),
                                demo: false,
                            };
                        }
                        Some(Row::Model { provider, model }) => {
                            let p = &self.registry.providers[*provider];
                            let choice = crate::providers::Choice {
                                provider: p.id.clone(),
                                model: p.models[*model].name.clone(),
                            };
                            if p.key.is_empty() {
                                self.message = format!("{} has no key yet · k adds one", p.name);
                                return Outcome::Keep;
                            }
                            self.registry.default = Some(choice.clone());
                            if let Err(e) = self.save() {
                                self.message = e;
                                return Outcome::Keep;
                            }
                            return Outcome::Use {
                                provider: Some(choice.provider),
                                model: choice.model,
                                demo: false,
                            };
                        }
                        None => {}
                    },
                    _ => {}
                }
                Outcome::Keep
            }
        }
    }
    fn submit(&mut self) -> Outcome {
        let View::Form(form) = &mut self.view else {
            return Outcome::Keep;
        };
        let provider = match form.provider() {
            Ok(p) => p,
            Err(e) => {
                form.error = e;
                return Outcome::Keep;
            }
        };
        if provider.key.trim().is_empty() && form.replacing.is_none() {
            form.error = "Paste the API key into the key field (it is hidden as you type)".into();
            form.focus = KEY;
            return Outcome::Keep;
        }
        let replacing = form.replacing.clone();
        let mut registry = self.registry.clone();
        match registry.upsert(provider, replacing.as_deref()) {
            Ok(id) => {
                if let Err(e) = registry.save(&self.path) {
                    form.error = format!("{e:#}");
                    return Outcome::Keep;
                }
                self.registry = registry;
                self.view = View::List;
                let rows = self.rows();
                self.index = rows
                    .iter()
                    .position(|r| {
                        matches!(r, Row::Model { provider, .. } | Row::Empty { provider }
                            if self.registry.providers[*provider].id == id)
                    })
                    .unwrap_or(0);
                let name = self
                    .registry
                    .find(&id)
                    .map(|p| p.name.clone())
                    .unwrap_or(id);
                Outcome::Saved(format!("Saved {name} · Enter uses a model · t tests it"))
            }
            Err(e) => {
                form.error = format!("{e:#}");
                Outcome::Keep
            }
        }
    }
    pub fn hints(&self) -> &'static str {
        match &self.view {
            View::List => {
                "Enter use · a add · e edit · k key · m models · t test · d delete · Esc close"
            }
            View::Form(_) => {
                "Tab/↑↓ field · ←→ preset or header · Ctrl+S or Enter on Models save · Esc back"
            }
            View::Confirm(_) => "y remove it · any other key keeps it",
        }
    }
    pub fn view(&self, width: usize) -> String {
        let mut out = String::new();
        match &self.view {
            View::Form(form) => {
                out += if form.replacing.is_some() {
                    "Edit a provider · Anthropic-compatible Messages API\n\n"
                } else {
                    "Add a provider · Anthropic-compatible Messages API\n\n"
                };
                for (i, label) in FIELDS.iter().enumerate() {
                    let marker = if form.focus == i { "›" } else { " " };
                    let value = match i {
                        PRESET => format!("‹ {} ›", PRESETS[form.preset].name),
                        AUTH => format!("‹ {} ›", form.auth.label()),
                        KEY if form.text[KEY].is_empty() => match &form.existing_key {
                            Some(mask) => format!("unchanged ({mask}) · type or paste to replace"),
                            None => "paste your key · it stays hidden".into(),
                        },
                        KEY => format!(
                            "{} ({} characters)",
                            "•".repeat(form.text[KEY].text.chars().count().min(24)),
                            form.text[KEY].text.chars().count()
                        ),
                        _ => form.text[i].text.clone(),
                    };
                    let cursor = if form.focus == i && !matches!(i, PRESET | AUTH | KEY) {
                        "▏"
                    } else {
                        ""
                    };
                    out += &format!("{marker} {label:11} {value}{cursor}\n");
                }
                out += "\nModels: comma-separated names; name=tokens sets a context window.\n";
                out += "The key is saved only in the private provider list beside your sessions\n";
                out += "(owner-only file) and sent only to this base URL. Use https; http only for localhost.\n";
                if !form.error.is_empty() {
                    out += &format!("\n! {}\n", form.error);
                }
            }
            View::Confirm(id) => {
                let name = self
                    .registry
                    .find(id)
                    .map(|p| p.name.as_str())
                    .unwrap_or(id);
                out += &format!(
                    "Remove {name}?\n\nIts key and model names are deleted from the provider list.\nConversations that used it keep their history and can switch to another model.\n"
                );
            }
            View::List => {
                out += "Choose the model for this conversation; new conversations start with it too.\n\n";
                let rows = self.rows();
                for (i, row) in rows.iter().enumerate() {
                    let marker = if i == self.index { "›" } else { " " };
                    let current = if self.is_current(row) {
                        "  ● in use"
                    } else {
                        ""
                    };
                    let line = match row {
                        Row::Env => format!(
                            "MiniMax · {} (from .env) · {}",
                            self.env_model,
                            if self.env_key { "key set" } else { "no key" }
                        ),
                        Row::Model { provider, model } => {
                            let p = &self.registry.providers[*provider];
                            let m = &p.models[*model];
                            format!(
                                "{} · {}{} · {}",
                                p.name,
                                m.name,
                                m.context
                                    .map(|c| format!(" · {}k context", c / 1000))
                                    .unwrap_or_default(),
                                masked(&p.key)
                            )
                        }
                        Row::Empty { provider } => format!(
                            "{} · no models yet · Enter adds one",
                            self.registry.providers[*provider].name
                        ),
                        Row::Demo => "Offline demo · scripted, no API calls".into(),
                        Row::Add => "+ Add a provider or API key".into(),
                    };
                    let line = crate::tools::clip(&line, width.saturating_sub(4).max(20));
                    out += &format!("{marker} {line}{current}\n");
                    if let Row::Model { provider, .. } | Row::Empty { provider } = row {
                        let p = &self.registry.providers[*provider];
                        let next_same = rows.get(i + 1).is_some_and(
                            |r| matches!(r, Row::Model { provider: q, .. } if q == provider),
                        );
                        if !next_same {
                            out += &format!("    {} · {}\n", p.base, p.auth.label());
                        }
                    }
                }
                if !self.message.is_empty() {
                    out += &format!("\n{}\n", self.message);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn panel(dir: &std::path::Path) -> Panel {
        Panel::open(
            Registry::path(dir),
            (None, "MiniMax-M2.7".into(), false),
            true,
            "MiniMax-M2.7".into(),
        )
    }
    #[test]
    fn adding_a_provider_hides_the_key_and_saves_it_privately() {
        let d = tempfile::tempdir().unwrap();
        let mut p = panel(d.path());
        assert_eq!(p.rows()[0], Row::Env);
        p.key(key(KeyCode::Char('a')));
        // The Anthropic preset fills the base URL, header and model names.
        let text = p.view(100);
        assert!(text.contains("https://api.anthropic.com") && text.contains("x-api-key"));
        assert!(text.contains("claude-opus-5"));
        // Saving without a key asks for one.
        assert_eq!(
            p.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Outcome::Keep
        );
        assert!(p.view(100).contains("Paste the API key"));
        p.paste("sk-ant-secret-value-9876\n");
        let shown = p.view(100);
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("••••"));
        assert!(matches!(
            p.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Outcome::Saved(_)
        ));
        let listed = p.view(100);
        assert!(listed.contains("Anthropic · claude-opus-5 · 1000k context · ••••9876"));
        assert!(!listed.contains("secret"));
        let saved = Registry::load(&Registry::path(d.path())).unwrap();
        assert_eq!(saved.providers[0].key, "sk-ant-secret-value-9876");
        // Enter uses the highlighted model and remembers it for new conversations.
        assert_eq!(
            p.key(key(KeyCode::Enter)),
            Outcome::Use {
                provider: Some("anthropic".into()),
                model: "claude-opus-5".into(),
                demo: false
            }
        );
        assert_eq!(
            Registry::load(&Registry::path(d.path()))
                .unwrap()
                .default
                .unwrap()
                .model,
            "claude-opus-5"
        );
    }
    #[test]
    fn custom_providers_edit_models_keep_keys_and_delete_on_confirmation() {
        let d = tempfile::tempdir().unwrap();
        let mut p = panel(d.path());
        p.key(key(KeyCode::Char('a')));
        p.key(key(KeyCode::Up)); // to the preset field
        p.key(key(KeyCode::Left)); // Anthropic -> Custom (wraps)
        assert!(p.view(100).contains("Custom"));
        p.key(key(KeyCode::Tab));
        p.paste("Local proxy");
        p.key(key(KeyCode::Tab));
        let base = &mut match &mut p.view {
            View::Form(f) => f,
            _ => unreachable!(),
        }
        .text[BASE];
        base.clear();
        p.paste("http://example.com");
        p.key(key(KeyCode::Tab));
        p.key(key(KeyCode::Tab));
        p.paste("local-key-0000");
        p.key(key(KeyCode::Tab));
        p.paste("qwen3-coder=262144, glm-4.6");
        assert_eq!(p.key(key(KeyCode::Enter)), Outcome::Keep);
        assert!(
            p.view(100)
                .contains("plain http is only allowed for localhost")
        );
        if let View::Form(f) = &mut p.view {
            f.text[BASE].set("http://127.0.0.1:4000");
        }
        assert!(matches!(p.key(key(KeyCode::Enter)), Outcome::Saved(_)));
        let listed = p.view(100);
        assert!(listed.contains("Local proxy · qwen3-coder · 262k context"));
        assert!(listed.contains("Local proxy · glm-4.6"));
        // Editing with an empty key field keeps the saved key.
        p.key(key(KeyCode::Char('m')));
        assert!(p.view(100).contains("unchanged (••••0000)"));
        if let View::Form(f) = &mut p.view {
            f.text[MODELS].set("qwen3-coder=262144");
        }
        p.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        let saved = Registry::load(&Registry::path(d.path())).unwrap();
        assert_eq!(saved.providers[0].key, "local-key-0000");
        assert_eq!(saved.providers[0].models.len(), 1);
        p.key(key(KeyCode::Char('d')));
        assert!(p.view(100).contains("Remove Local proxy?"));
        assert!(matches!(p.key(key(KeyCode::Char('y'))), Outcome::Saved(_)));
        assert!(
            Registry::load(&Registry::path(d.path()))
                .unwrap()
                .providers
                .is_empty()
        );
    }
}
