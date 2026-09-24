//! Model providers you add yourself, with their API keys.
//!
//! The registry lives in `~/.config/aster/providers.json`, created with owner-only
//! permissions. A key is only ever sent to the base URL of the provider it belongs to,
//! and it never appears in sessions, transcripts, exports, notices or diagnostics.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    path::{Path, PathBuf},
};

/// How the key is presented to the provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Auth {
    /// `Authorization: Bearer KEY` (MiniMax and most Anthropic-compatible services).
    #[default]
    Bearer,
    /// `x-api-key: KEY` (Anthropic's own API).
    XApiKey,
}
impl Auth {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bearer => "Authorization: Bearer",
            Self::XApiKey => "x-api-key",
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    /// Context window in tokens, when known; the meter and auto-compact use it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub base: String,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub models: Vec<Model>,
}
// Never let a key reach a log line, panic message or test failure.
impl fmt::Debug for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Provider")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("base", &self.base)
            .field("auth", &self.auth)
            .field("key", &masked(&self.key))
            .field(
                "models",
                &self.models.iter().map(|m| &m.name).collect::<Vec<_>>(),
            )
            .finish()
    }
}
impl Provider {
    pub fn context(&self, model: &str) -> Option<u64> {
        self.models
            .iter()
            .find(|m| m.name == model)
            .and_then(|m| m.context)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub providers: Vec<Provider>,
    /// Used for new conversations.
    #[serde(default)]
    pub default: Option<Choice>,
}

/// A starting point for the add form. Base URLs and model names can be edited.
pub struct Preset {
    pub name: &'static str,
    pub base: &'static str,
    pub auth: Auth,
    pub models: &'static [(&'static str, Option<u64>)],
}
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Anthropic",
        base: "https://api.anthropic.com",
        auth: Auth::XApiKey,
        models: &[
            ("claude-opus-5", Some(1_000_000)),
            ("claude-sonnet-5", Some(1_000_000)),
            ("claude-haiku-4-5", Some(200_000)),
        ],
    },
    Preset {
        name: "MiniMax",
        base: "https://api.minimaxi.com/anthropic",
        auth: Auth::Bearer,
        models: &[("MiniMax-M2.7", None)],
    },
    Preset {
        name: "Custom (Anthropic-compatible)",
        base: "https://",
        auth: Auth::Bearer,
        models: &[],
    },
];

pub fn masked(key: &str) -> String {
    let key = key.trim();
    if key.is_empty() {
        return "no key".into();
    }
    let tail = key
        .chars()
        .rev()
        .take(if key.chars().count() >= 12 { 4 } else { 0 })
        .collect::<Vec<_>>();
    format!("••••{}", tail.into_iter().rev().collect::<String>())
}

/// Accept https anywhere, and plain http only on this machine (a local proxy).
pub fn validate_base(base: &str) -> Result<String> {
    let base = base.trim().trim_end_matches('/');
    let url =
        reqwest::Url::parse(base).context("Use a full URL such as https://api.example.com")?;
    let host = url
        .host_str()
        .context("The URL has no host")?
        .to_ascii_lowercase();
    let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "::1");
    match url.scheme() {
        "https" => {}
        "http" if local => {}
        "http" => bail!("Use https; plain http is only allowed for localhost"),
        _ => bail!("Use an https:// URL"),
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("Put the key in the key field, not in the URL");
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("The base URL cannot have a query or fragment");
    }
    Ok(base.to_string())
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "provider".into()
    } else {
        out
    }
}

impl Registry {
    /// Beside the private session store, so `--state-dir` keeps a separate list.
    pub fn path(state: &Path) -> PathBuf {
        state.join("providers.json")
    }
    /// Point a turn's configuration at the conversation's provider. `None` keeps the
    /// MiniMax settings from `.env` or the environment.
    pub fn apply(
        &self,
        cfg: &mut crate::config::Config,
        provider: Option<&str>,
        model: &str,
    ) -> Result<()> {
        let Some(id) = provider else {
            return Ok(());
        };
        let p = self.find(id).with_context(|| {
            format!("Provider {id} is no longer configured; choose a model with /models")
        })?;
        cfg.base = p.base.clone();
        cfg.key = p.key.clone();
        cfg.auth = p.auth;
        cfg.provider = p.name.clone();
        if let Some(window) = p.context(model) {
            cfg.limits.context_tokens = window;
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).context("Cannot read the provider list"),
        };
        if bytes.len() > 256_000 {
            bail!("The provider list exceeds 256 KB");
        }
        let registry: Self =
            serde_json::from_slice(&bytes).context("The provider list is not valid JSON")?;
        for p in &registry.providers {
            validate_base(&p.base).with_context(|| format!("Provider {}", p.name))?;
        }
        Ok(registry)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        let dir = path.parent().context("Invalid provider list path")?;
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        crate::session::atomic_json(path, self)
    }
    pub fn find(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.id == id)
    }
    pub fn find_mut(&mut self, id: &str) -> Option<&mut Provider> {
        self.providers.iter_mut().find(|p| p.id == id)
    }
    /// Add or replace a provider; returns its id. Names stay unique.
    pub fn upsert(&mut self, mut provider: Provider, replacing: Option<&str>) -> Result<String> {
        let name = provider.name.trim().to_string();
        if name.is_empty() || name.chars().count() > 60 {
            bail!("Give the provider a name of 1–60 characters");
        }
        provider.name = name;
        provider.base = validate_base(&provider.base)?;
        provider.key = provider.key.trim().to_string();
        if provider.key.len() > 4096 || provider.key.chars().any(char::is_control) {
            bail!("That key has invalid characters or is too long");
        }
        provider.models.retain(|m| !m.name.trim().is_empty());
        for m in &mut provider.models {
            m.name = m.name.trim().to_string();
            if m.name.len() > 120 || m.name.chars().any(|c| c.is_control() || c == ' ') {
                bail!("Model names are up to 120 characters, without spaces");
            }
        }
        provider.models.dedup_by(|a, b| a.name == b.name);
        if self.providers.iter().any(|p| {
            p.name.eq_ignore_ascii_case(&provider.name) && Some(p.id.as_str()) != replacing
        }) {
            bail!("A provider named {} already exists", provider.name);
        }
        if let Some(old) = replacing.and_then(|id| self.find_mut(id)) {
            provider.id = old.id.clone();
            // An empty key field while editing keeps the saved key.
            if provider.key.is_empty() {
                provider.key = old.key.clone();
            }
            *old = provider.clone();
            return Ok(provider.id);
        }
        let base = slug(&provider.name);
        let mut id = base.clone();
        let mut n = 2;
        while self.find(&id).is_some() {
            id = format!("{base}-{n}");
            n += 1;
        }
        provider.id = id.clone();
        self.providers.push(provider);
        Ok(id)
    }
    pub fn remove(&mut self, id: &str) {
        self.providers.retain(|p| p.id != id);
        if self.default.as_ref().is_some_and(|c| c.provider == id) {
            self.default = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn provider(name: &str, key: &str) -> Provider {
        Provider {
            id: String::new(),
            name: name.into(),
            base: "https://api.example.com/anthropic/".into(),
            auth: Auth::Bearer,
            key: key.into(),
            models: vec![Model {
                name: "model-a".into(),
                context: Some(128_000),
            }],
        }
    }
    #[test]
    fn keys_are_masked_everywhere_they_could_be_printed() {
        let p = provider("Example", "sk-live-1234567890abcd");
        assert_eq!(masked(&p.key), "••••abcd");
        assert_eq!(masked("short"), "••••");
        assert_eq!(masked(""), "no key");
        let debug = format!("{p:?}");
        assert!(!debug.contains("1234567890"), "{debug}");
    }
    #[test]
    fn base_urls_must_be_https_or_local() {
        assert_eq!(
            validate_base("https://api.example.com/anthropic/").unwrap(),
            "https://api.example.com/anthropic"
        );
        assert!(validate_base("http://127.0.0.1:4000").is_ok());
        assert!(validate_base("http://localhost:4000/v1").is_ok());
        for bad in [
            "http://api.example.com",
            "ftp://example.com",
            "https://user:pass@example.com",
            "https://example.com/?key=abc",
            "https://example.com/#x",
            "api.example.com",
        ] {
            assert!(validate_base(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn registry_saves_privately_and_edits_keep_the_saved_key() {
        let d = tempfile::tempdir().unwrap();
        let path = Registry::path(d.path());
        let mut r = Registry::default();
        let id = r
            .upsert(provider("My Proxy", "secret-key-0001"), None)
            .unwrap();
        assert_eq!(id, "my-proxy");
        assert!(r.upsert(provider("my proxy", "x"), None).is_err());
        let second = r.upsert(provider("My Proxy 2", ""), None).unwrap();
        assert_eq!(second, "my-proxy-2");
        let mut edited = provider("My Proxy", "");
        edited.models.push(Model {
            name: "model-b".into(),
            context: None,
        });
        r.upsert(edited, Some("my-proxy")).unwrap();
        assert_eq!(r.find("my-proxy").unwrap().key, "secret-key-0001");
        assert_eq!(r.find("my-proxy").unwrap().models.len(), 2);
        assert_eq!(
            r.find("my-proxy").unwrap().context("model-a"),
            Some(128_000)
        );
        r.default = Some(Choice {
            provider: "my-proxy".into(),
            model: "model-b".into(),
        });
        r.save(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        let loaded = Registry::load(&path).unwrap();
        assert_eq!(loaded, r);
        let mut r = loaded;
        r.remove("my-proxy");
        assert!(r.default.is_none());
        assert!(
            Registry::load(&d.path().join("missing.json"))
                .unwrap()
                .providers
                .is_empty()
        );
        let mut bad = provider("Bad", "k");
        bad.models = vec![Model {
            name: "has space".into(),
            context: None,
        }];
        assert!(r.upsert(bad, None).is_err());
    }
}
