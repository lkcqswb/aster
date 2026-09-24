use anyhow::Result;
use aster::{
    config::{Cli, Config},
    session::Store,
};
use clap::Parser;
fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load(&cli)?;
    if let Some(dir) = &cli.live2d_probe {
        return aster::live2d::probe(&cfg, dir);
    }
    let store = Store::open(&cfg.state)?;
    if let Some(path) = cli.screenshot.clone() {
        return aster::ui::screenshot(cfg, cli, store, &path);
    }
    if cli.prompt.is_some() {
        aster::ui::headless(cfg, cli, store)
    } else {
        aster::ui::run(cfg, cli, store)
    }
}
