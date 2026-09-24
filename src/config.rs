use anyhow::{Context, Result, bail};
use clap::Parser;
use std::{collections::HashMap, path::PathBuf};

#[derive(Parser, Debug, Clone)]
#[command(
    version,
    about = "Aster · work beside 弄玉",
    long_about = "A Rust terminal agent with sessions, AGENTS.md, slash commands and a live Live2D companion."
)]
pub struct Cli {
    #[arg(short, long, default_value = ".")]
    /// Project to work in; defaults to the current directory
    pub project: PathBuf,
    #[arg(long)]
    /// Use a separate private session store
    pub state_dir: Option<PathBuf>,
    #[arg(long)]
    /// Resume a saved session by its full ID
    pub resume: Option<String>,
    #[arg(long = "continue")]
    /// Continue the latest conversation in this project
    pub continue_last: bool,
    #[arg(long)]
    /// Use the offline demo instead of a model API
    pub demo: bool,
    #[arg(long)]
    /// Start without the Live2D renderer
    pub no_live2d: bool,
    #[arg(long, default_value = "auto", value_parser = ["auto", "iterm", "kitty", "halfblocks", "off"])]
    /// Terminal image protocol; auto detects supported terminals
    pub graphics: String,
    #[arg(long)]
    /// MiniMax model name
    pub model: Option<String>,
    #[arg(long)]
    /// Directory containing the local model and vendor SDK assets
    pub pet_dir: Option<PathBuf>,
    #[arg(long)]
    /// Chrome or Chromium executable for the private renderer
    pub chrome: Option<PathBuf>,
    #[arg(long)]
    /// Run one turn without opening the terminal interface
    pub prompt: Option<String>,
    #[arg(long, default_value = "ask", value_parser = ["ask", "allow", "deny"])]
    /// Approval policy for file writes and shell commands
    pub permissions: String,
    #[arg(long)]
    /// Capture and verify changing Live2D frames in this directory
    pub live2d_probe: Option<PathBuf>,
    #[arg(long)]
    /// Save a preview of the terminal layout as SVG
    pub screenshot: Option<PathBuf>,
    #[arg(long, default_value_t = 132)]
    /// Width of the SVG preview in terminal cells
    pub width: u16,
    #[arg(long, default_value_t = 42)]
    /// Height of the SVG preview in terminal cells
    pub height: u16,
}

#[derive(Clone)]
pub struct Config {
    pub home: PathBuf,
    pub project: PathBuf,
    pub state: PathBuf,
    pub key: String,
    pub base: String,
    pub model: String,
    pub pet: PathBuf,
    pub chrome: PathBuf,
}
impl Config {
    pub fn load(cli: &Cli) -> Result<Self> {
        let home = std::env::var_os("ASTER_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        let user = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
        let env = read_env(&home.join(".env"));
        let value = |name: &str, default: &str| {
            std::env::var(name)
                .ok()
                .or_else(|| env.get(name).cloned())
                .unwrap_or_else(|| default.into())
        };
        let project = cli
            .project
            .canonicalize()
            .context("Project directory not found")?;
        if !project.is_dir() {
            bail!("Project must be a directory");
        }
        let base = value("ANTHROPIC_BASE_URL", "https://api.minimaxi.com/anthropic");
        let url = reqwest::Url::parse(&base)?;
        if url.scheme() != "https"
            || !matches!(
                url.host_str(),
                Some("api.minimaxi.com" | "api.minimax.cn" | "api.minimax.io")
            )
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("Use an official MiniMax HTTPS endpoint");
        }
        Ok(Self {
            home,
            project,
            state: cli
                .state_dir
                .clone()
                .unwrap_or_else(|| user.join(".local/share/aster")),
            key: value("ANTHROPIC_AUTH_TOKEN", &value("ANTHROPIC_API_KEY", "")),
            base,
            model: cli
                .model
                .clone()
                .unwrap_or_else(|| value("MINIMAX_MODEL", "MiniMax-M2.7")),
            pet: cli
                .pet_dir
                .clone()
                .or_else(|| std::env::var_os("ASTER_PET_DIR").map(PathBuf::from))
                .unwrap_or_else(|| user.join("desktop-pet/assets")),
            chrome: cli
                .chrome
                .clone()
                .or_else(|| std::env::var_os("ASTER_CHROME").map(PathBuf::from))
                .unwrap_or_else(|| {
                    PathBuf::from(if cfg!(target_os = "macos") {
                        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
                    } else {
                        "/usr/bin/chromium"
                    })
                }),
        })
    }
}
fn read_env(path: &std::path::Path) -> HashMap<String, String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (k, v) = line.trim().split_once('=')?;
            matches!(
                k.trim(),
                "ANTHROPIC_AUTH_TOKEN"
                    | "ANTHROPIC_API_KEY"
                    | "ANTHROPIC_BASE_URL"
                    | "MINIMAX_MODEL"
            )
            .then(|| {
                (
                    k.trim().to_string(),
                    v.trim().trim_matches(['\'', '"']).to_string(),
                )
            })
        })
        .collect()
}
