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
    #[arg(long, default_value_t = 2048, value_parser = clap::value_parser!(u32).range(512..=4096))]
    /// Maximum Live2D texture edge for this renderer; original assets stay unchanged
    pub texture_size: u32,
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
    #[arg(long, requires = "screenshot", value_parser = ["skills", "context", "work", "review", "prompts", "output", "checks", "files", "find", "together", "checkpoint", "history", "tasks"])]
    /// Open a read-only panel in an SVG preview; never submits a model request
    pub preview_panel: Option<String>,
    #[arg(long, env = "ASTER_MAX_REQUESTS", default_value_t = 40, value_parser = clap::value_parser!(u32).range(1..=500))]
    /// Model requests allowed in one turn
    pub max_requests: u32,
    #[arg(long, env = "ASTER_MAX_TOOLS", default_value_t = 120, value_parser = clap::value_parser!(u32).range(1..=2000))]
    /// Tool calls allowed in one turn
    pub max_tools: u32,
    #[arg(long, env = "ASTER_TURN_SECONDS", default_value_t = 900, value_parser = clap::value_parser!(u64).range(10..=7200))]
    /// Active seconds allowed in one turn; decision waits pause this timer
    pub turn_seconds: u64,
    #[arg(long, env = "ASTER_MAX_OUTPUT_TOKENS", default_value_t = 8192, value_parser = clap::value_parser!(u64).range(256..=131072))]
    /// Output tokens allowed in one model request
    pub max_output_tokens: u64,
    #[arg(long, env = "ASTER_TURN_OUTPUT_TOKENS", default_value_t = 64000, value_parser = clap::value_parser!(u64).range(256..=1000000))]
    /// Output tokens allowed across one turn
    pub turn_output_tokens: u64,
    #[arg(long, env = "ASTER_CONTEXT_WINDOW", default_value_t = 200000, value_parser = clap::value_parser!(u64).range(8000..=2000000))]
    /// Model context window in tokens, used for the context meter and auto-compact
    pub context_window: u64,
    #[arg(long, env = "ASTER_AUTO_COMPACT", default_value_t = 80, value_parser = clap::value_parser!(u8).range(0..=95))]
    /// Auto-compact when context reaches this percent of the window; 0 disables it
    pub auto_compact: u8,
    #[arg(long, default_value_t = 132)]
    /// Width of the SVG preview in terminal cells
    pub width: u16,
    #[arg(long, default_value_t = 42)]
    /// Height of the SVG preview in terminal cells
    pub height: u16,
}

/// Work limits for one user turn. They bound effort, not currency.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    pub requests: u32,
    pub tools: u32,
    pub active_secs: u64,
    pub request_output_tokens: u64,
    pub turn_output_tokens: u64,
    pub shell_secs: u64,
    pub context_tokens: u64,
    /// Percent of the context window that triggers auto-compact; 0 disables it.
    pub auto_compact: u8,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            requests: 40,
            tools: 120,
            active_secs: 900,
            request_output_tokens: 8192,
            turn_output_tokens: 64_000,
            shell_secs: 600,
            context_tokens: 200_000,
            auto_compact: 80,
        }
    }
}
impl Limits {
    pub fn from_cli(cli: &Cli) -> Self {
        Self {
            requests: cli.max_requests,
            tools: cli.max_tools,
            active_secs: cli.turn_seconds,
            request_output_tokens: cli.max_output_tokens,
            turn_output_tokens: cli.turn_output_tokens.max(cli.max_output_tokens),
            shell_secs: 600,
            context_tokens: cli.context_window,
            auto_compact: cli.auto_compact,
        }
    }
    /// Refuse to send a request whose serialized context is far beyond the window.
    pub fn request_bytes(&self) -> usize {
        (self.context_tokens as usize).saturating_mul(5)
    }
    pub fn describe(&self) -> String {
        format!(
            "{} model requests · {} tool calls · {} active seconds\n{} output tokens per request · {} per turn\nShell commands up to {} seconds each\nContext window {} tokens · auto-compact {}",
            self.requests,
            self.tools,
            self.active_secs,
            self.request_output_tokens,
            self.turn_output_tokens,
            self.shell_secs,
            self.context_tokens,
            if self.auto_compact == 0 {
                "off".to_string()
            } else {
                format!("at {}%", self.auto_compact)
            }
        )
    }
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
    pub texture_size: u32,
    pub limits: Limits,
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
            texture_size: cli.texture_size,
            limits: Limits::from_cli(cli),
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn texture_limits_are_validated_before_startup() {
        assert_eq!(Cli::try_parse_from(["aster"]).unwrap().texture_size, 2048);
        assert_eq!(
            Cli::try_parse_from(["aster", "--texture-size", "4096"])
                .unwrap()
                .texture_size,
            4096
        );
        assert!(Cli::try_parse_from(["aster", "--texture-size", "511"]).is_err());
        assert!(Cli::try_parse_from(["aster", "--texture-size", "4097"]).is_err());
    }
    #[test]
    fn turn_limits_default_higher_and_stay_bounded() {
        let cli = Cli::try_parse_from(["aster"]).unwrap();
        assert_eq!(Limits::from_cli(&cli), Limits::default());
        let cli = Cli::try_parse_from([
            "aster",
            "--max-requests",
            "80",
            "--max-output-tokens",
            "16000",
            "--turn-output-tokens",
            "1000",
            "--auto-compact",
            "0",
        ])
        .unwrap();
        let limits = Limits::from_cli(&cli);
        assert_eq!(limits.requests, 80);
        // A turn can always fit at least one full request.
        assert_eq!(limits.turn_output_tokens, 16000);
        assert!(limits.describe().contains("auto-compact off"));
        assert!(Cli::try_parse_from(["aster", "--max-requests", "0"]).is_err());
        assert!(Cli::try_parse_from(["aster", "--auto-compact", "99"]).is_err());
    }
}
