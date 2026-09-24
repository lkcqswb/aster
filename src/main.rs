use anyhow::Result;
use aster::{
    config::{Cli, Config},
    session::Store,
};
use clap::Parser;
fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(path) = &cli.check_companion {
        let profile = aster::companion::Profile::load(Some(path))?;
        println!(
            "Companion v{} · {} · {} emotions · {} motions · {} bindings",
            profile.version,
            profile.name,
            profile.emotions.len(),
            profile.motions.len(),
            profile.bindings.len()
        );
        return Ok(());
    }
    let _lifetime = aster::lifecycle::install()?;
    let cfg = Config::load(&cli)?;
    if let Some(dir) = &cli.live2d_probe {
        return aster::live2d::probe(
            &cfg,
            dir,
            cli.probe_emotion.as_deref(),
            cli.probe_motion.as_deref(),
        );
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
