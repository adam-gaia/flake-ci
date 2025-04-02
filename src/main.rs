use anyhow::{bail, Result};
use clap::Parser;
use log::debug;
use std::env;

mod config;
use config::Config;

mod app;
use app::App;

mod nix;

const CONFIG_FILE_NAME: &str = "nix-ci.toml";

// TODO: make this into a lib crate. Also add a bin that calls the function and prints the system
fn system() -> &'static str {
    match (env::consts::ARCH, env::consts::OS) {
        ("x86_64", "linux") => "x86_64-linux",
        ("aarch64", "linux") => "aarch64-linux",
        ("x86_64", "macos") => "x86_64-darwin",
        ("aarch64", "macos") => "aarch64-darwin",
        ("x86", "windows") => "i686-windows",
        ("x86_64", "windows") => "x86_64-windows",
        ("aarch64", "windows") => "aarch64-windows",
        _ => "unknown",
    }
}

#[derive(Debug, Parser)]
struct Cli {
    #[clap(short, long)]
    dry_run: bool,
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Cli::parse();

    let cwd = env::current_dir()?;
    let config_file = cwd.join(CONFIG_FILE_NAME);
    let config = if config_file.is_file() {
        Config::from_file(&config_file)?
    } else {
        Config::default()
    };

    let app = App::with_config(config)?;
    app.run(args.dry_run)?;

    Ok(())
}
