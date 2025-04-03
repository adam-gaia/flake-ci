use anyhow::{bail, Result};
use clap::Parser;
use log::debug;
use std::env;

mod config;
use config::{Config, System};

mod app;
use app::App;

mod nix;

const CONFIG_FILE_NAME: &str = "flake-ci.toml";

// TODO: make this into a lib crate. Also add a bin that calls the function and prints the system
fn system() -> Result<System> {
    let arch = env::consts::ARCH;
    let os = env::consts::OS;
    let system = match (arch, os) {
        ("x86_64", "linux") => System::x86_linux(),
        ("aarch64", "linux") => System::arm_linux(),
        ("x86_64", "macos") => System::x86_darwin(),
        ("aarch64", "macos") => System::arm_darwin(),
        ("x86", "windows") => System::x86_windows(),
        ("x86_64", "windows") => System::x86_windows(),
        ("aarch64", "windows") => System::arm_windows(),
        _ => bail!("Unknown system: arch: '{arch}, os: '{os}'"),
    };
    Ok(system)
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

    let system = system()?;
    let app = App::with_config(cwd, system, config)?;
    app.run(args.dry_run)?;

    Ok(())
}
