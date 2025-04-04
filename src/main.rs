use anyhow::{bail, Result};
use clap::Parser;
use std::{env, fs, path::PathBuf};

mod config;
use config::{Config, System};

mod app;
use app::App;

mod nix;

const MAX_WIDTH: usize = 100;
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
        ("x86" | "x86_64", "windows") => System::x86_windows(),
        ("aarch64", "windows") => System::arm_windows(),
        _ => bail!("Unknown system: arch: '{arch}, os: '{os}'"),
    };
    Ok(system)
}

#[derive(Debug, Parser)]
struct Cli {
    #[clap(long)]
    dry_run: bool,
    #[clap(long)]
    dir: Option<PathBuf>,
}

fn main() -> Result<()> {
    env_logger::init();
    let cwd = env::current_dir()?;

    let args = Cli::parse();
    let working_dir = match args.dir {
        Some(dir) => {
            let dir = fs::canonicalize(dir)?;
            env::set_current_dir(&dir)?;
            dir
        }
        None => cwd.clone(),
    };

    let config_file = working_dir.join(CONFIG_FILE_NAME);
    let config = if config_file.is_file() {
        Config::from_file(&config_file)?
    } else {
        Config::default()
    };

    let system = system()?;
    let width = match term_size::dimensions() {
        Some((w, _)) => std::cmp::min(w, MAX_WIDTH),
        None => MAX_WIDTH,
    };

    let app = App::with_config(cwd, &working_dir, system, width, config)?;
    if !app.run(args.dry_run)? {
        std::process::exit(1);
    }
    Ok(())
}
