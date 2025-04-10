use anyhow::{bail, Result};
use clap::Parser;
use log::{debug, warn};
use std::path::Path;
use std::{env, fs, path::PathBuf};
mod config;
use crate::model::System;
use config::Config;

mod drvtree;
use drvtree::App;

mod graph;
mod model;
mod nix;
mod util;

const CONFIG_FILE_NAME: &str = "flake-ci.toml";
const MAX_WIDTH: usize = 100;

// TODO: make this into a lib crate. Also add a bin that calls the function and prints the system
fn current_system() -> Result<System> {
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
    /// Print what would be done without doing anything
    #[clap(long)]
    dry_run: bool,
    /// Project directory to operate on
    #[clap(long)]
    dir: Option<PathBuf>,
    /// Do not publish build artifacts to cachix
    #[clap(long)]
    no_publish: bool,
    /// TODO
    #[clap(long)]
    no_cachix: bool,
    /// TODO
    #[clap(long)]
    no_fmt: bool,
    /// Print the build order without doing anything
    #[clap(long)]
    print_build_order: bool,
    #[clap(short, long)]
    num_threads: Option<usize>,
}

#[derive(Debug, Clone)]
struct CachixSettings {
    cachix: PathBuf,
    publish: bool,
    cache_name: String,
}

impl CachixSettings {
    pub fn new(cachix: PathBuf, publish: bool, cache_name: String) -> Self {
        Self {
            cachix,
            publish,
            cache_name,
        }
    }

    pub fn cachix_path(&self) -> &Path {
        &self.cachix
    }

    pub fn publish(&self) -> bool {
        self.publish
    }

    pub fn cache_name(&self) -> &str {
        &self.cache_name
    }
}

#[derive(Debug)]
struct Settings {
    cwd: PathBuf,
    nix: PathBuf,
    cachix_settings: Option<CachixSettings>,
    current_system: System,
    dry_run: bool,
    no_fmt: bool,
    print_build_order: bool,
    width: usize,
    num_threads: usize,
    repo_config: Config,
}

impl Settings {
    pub fn working_dir(&self) -> &Path {
        &self.cwd
    }

    pub fn nix(&self) -> &Path {
        &self.nix
    }

    pub fn cachix_settings(&self) -> Option<&CachixSettings> {
        self.cachix_settings.as_ref()
    }

    pub fn dry_run(&self) -> bool {
        self.dry_run
    }

    pub fn current_system(&self) -> System {
        self.current_system
    }

    pub fn print_build_order(&self) -> bool {
        self.print_build_order
    }

    pub fn num_threads(&self) -> usize {
        self.num_threads
    }

    pub fn repo_config(&self) -> &Config {
        &self.repo_config
    }
}

impl Settings {
    fn new(args: Cli, cwd: PathBuf) -> Result<Self> {
        let current_system = current_system()?;

        let nix = which("nix")?;

        // TODO: search back for repo root instead of using cwd
        let repo_config_file = cwd.join(CONFIG_FILE_NAME);
        let repo_config = if repo_config_file.is_file() {
            Config::from_file(&repo_config_file)?
        } else {
            warn!("No config file found, using default config");
            Config::default()
        };

        debug!("{repo_config:#?}");

        let cachix_settings = if args.no_cachix {
            None
        } else {
            let cachix = which("cachix")?;
            let no_publish = args.no_publish;

            let cache_name = repo_config
                .cache()
                .expect("Config file doesn't have cachix.cache-name set")
                .clone();

            Some(CachixSettings {
                cachix,
                publish: !no_publish,
                cache_name,
            })
        };

        let dry_run = args.dry_run;
        let mut no_fmt = args.no_fmt;

        let print_build_order = args.print_build_order;
        if print_build_order {
            no_fmt = true;
        };

        let width = match term_size::dimensions() {
            Some((w, _)) => std::cmp::min(w, MAX_WIDTH),
            None => MAX_WIDTH,
        };

        let num_threads = match args.num_threads {
            Some(n) => n,
            None => rayon::current_num_threads(),
        };

        Ok(Self {
            cwd,
            current_system,
            nix,
            cachix_settings,
            dry_run,
            no_fmt,
            print_build_order,
            width,
            num_threads,
            repo_config,
        })
    }
}

fn which(name: &str) -> Result<PathBuf> {
    let Ok(path) = which::which(name) else {
        bail!("Unable to find {name} on the $PATH");
    };
    Ok(path)
}

fn main() -> Result<()> {
    env_logger::init();
    let cwd = env::current_dir()?;

    let args = Cli::parse();
    let cwd = match args.dir {
        Some(ref dir) => {
            let dir = fs::canonicalize(dir)?;
            env::set_current_dir(&dir)?;
            dir
        }
        None => cwd.clone(),
    };

    let settings = Settings::new(args, cwd)?;

    // TODO: layered config to merge args into config, then only need to pass config
    let app = App::with_settings(settings)?;
    if !app.run()? {
        std::process::exit(1);
    }
    Ok(())
}
