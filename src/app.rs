use crate::config::{Config, System};
use crate::nix::{run, run_stream};
use anyhow::{bail, Result};
use log::{debug, info, warn};
use owo_colors::{OwoColorize, Style};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;
use which::which;
mod summary;
use summary::Summary;

const CACHIX_AUTH_KEY: &str = "CACHIX_AUTH_TOKEN";
const CACHIX_SIGNING_KEY: &str = "CACHIX_SIGNING_KEY";

#[derive(Debug)]
pub enum Status {
    Skipped,
    Success,
    Fail,
}

fn get_version(bin: &Path) -> Result<String> {
    let output = run(bin, &["--version"])?;
    let version = output.lines().next().unwrap();
    Ok(version.to_string())
}

fn nix_version(nix: &Path) -> Result<String> {
    get_version(nix)
}

fn cachix_version(cachix: &Path) -> Result<String> {
    get_version(cachix)
}

fn git_revision() -> Result<String> {
    let git = which("git")?;
    let commit_hash = run(&git, &["rev-parse", "--short", "HEAD"])?;
    let dirty = if run(&git, &["status", "--porcelain"])?.is_empty() {
        ""
    } else {
        " (dirty)"
    };
    let revision = format!("{commit_hash}{dirty}");
    Ok(revision)
}

fn env_set(key: &str) -> bool {
    env::var(key).is_ok()
}

fn setup_cachix(cachix: &Path, cache: &str, dry_run: bool) -> Result<()> {
    if !(env_set(CACHIX_AUTH_KEY) || env_set(CACHIX_SIGNING_KEY)) {
        bail!("Neither env var {CACHIX_AUTH_KEY} or {CACHIX_SIGNING_KEY} set. At least one is required for cachix support");
    }

    info!("Using cachix");

    run_stream(cachix, &["use", cache], None, dry_run)?;
    Ok(())
}

#[derive(Debug)]
pub struct App {
    cwd: PathBuf,
    output_dir: PathBuf,
    nix_result_dir: PathBuf,
    config: Config,
    nix: PathBuf,
    cachix: Option<PathBuf>,
    system: System,
    width: usize,
}

impl App {
    pub fn with_config(
        cwd: PathBuf,
        working_dir: &Path,
        system: System,
        width: usize,
        config: Config,
    ) -> Result<Self> {
        let output_dir = working_dir.join(config.artifact_dir());
        let nix_result_dir = working_dir.join("result");
        let Ok(nix) = which::which("nix") else {
            bail!("Unable to find nix on the $PATH");
        };

        let cachix = match config.cache() {
            Some(_) => {
                let Ok(cachix) = which::which("cachix") else {
                    bail!("Unable to find cachix on the $PATH (config has cachix set)");
                };
                Some(cachix)
            }
            None => None,
        };

        Ok(Self {
            cwd,
            output_dir,
            nix_result_dir,
            config,
            nix,
            cachix,
            system,
            width,
        })
    }

    fn attributes(&self, ttype: &str, system: System) -> Result<Vec<String>> {
        let args = &[
            "eval",
            &format!(".#{ttype}.{system}"),
            "--apply",
            "builtins.attrNames",
            "--json",
        ];
        let stdout = run(&self.nix, args)?;
        let attributes: Vec<String> = serde_json::from_str(&stdout)?;
        Ok(attributes)
    }

    fn derivation_path(&self, ttype: &str, system: System, attribute: &str) -> Result<String> {
        let args = &[
            "eval",
            &format!(".#{ttype}.{system}.{attribute}"),
            "--apply",
            "pkg: pkg.drvPath",
            "--raw",
        ];
        let path = run(&self.nix, args)?;
        Ok(path)
    }

    fn build(&self, path: &str, dry_run: bool) -> Result<Status> {
        let nix_args = &[
            "build",
            &format!("{path}^*"),
            "--log-lines",
            "0",
            "--print-build-logs",
            "--print-out-paths",
        ];

        let env = Some(self.config.env());

        let status = if self.config.publish() {
            // Run nix build under cachix. Cachix will push all built paths
            let nix = self.nix.display().to_string();
            let mut args = vec!["watch-exec", &self.config.cache().unwrap(), "--", &nix];
            args.extend_from_slice(nix_args);
            run_stream(&self.cachix.clone().unwrap(), &args, env, dry_run)?
        } else {
            run_stream(&self.nix, nix_args, env, dry_run)?
        };
        Ok(status)
    }

    pub fn build_all(&self, dry_run: bool, summary: &mut Summary) -> Result<bool> {
        let mut all_succeeded = true;

        for system in &self.config.systems() {
            if system != &self.system {
                // TODO: cross compiling??
                warn!("Skipping system {}", system);
                continue;
            }

            for output in self.config.build_outputs() {
                debug!("Output type: {output}");

                let Ok(attributes) = self.attributes(output, *system) else {
                    warn!("No such entry: .#{output}");
                    summary.skip_output(output);
                    continue;
                };

                for attribute in &attributes {
                    debug!("Attr: {attribute}");

                    let path = self.derivation_path(output, *system, attribute)?;
                    debug!("Path: {path}");

                    let derivation = format!(".#{output}.{system}.{attribute}");
                    info!("Building {derivation}");
                    let status = self.build(&path, dry_run)?;

                    info!("Done building {derivation}");
                    match status {
                        Status::Skipped => {
                            summary.register_skip(output, derivation);
                        }
                        Status::Fail => {
                            all_succeeded = false;
                            let log_command = format!("`nix log {path}`");
                            summary.register_fail(output, derivation, log_command);
                        }
                        Status::Success => {
                            let artifact = if !dry_run
                                && self.config.save_artifact(output, *system, attribute)
                            {
                                debug!("Saving artifacts from {}", &derivation);
                                let artifact = &self.nix_result_dir;
                                if !artifact.is_symlink() {
                                    bail!("Error: todo better error message");
                                }

                                let artifact = fs::canonicalize(artifact)?;
                                debug!("artifact to save: {}", artifact.display());

                                let link = self.output_dir.join(&derivation);
                                debug!("link: {}", link.display());
                                symlink(&artifact, &link)?;

                                Some(link)
                            } else {
                                None
                            };

                            summary.register_success(output, derivation.clone(), artifact);
                        }
                    }
                }
            }
        }
        Ok(all_succeeded)
    }

    pub fn run(&self, dry_run: bool) -> Result<bool> {
        let nix_version = nix_version(&self.nix)?;
        let git_revision = git_revision()?;

        let cachix_version = match &self.cachix {
            Some(cachix) => {
                info!("Setting up nix to work with cachix");
                setup_cachix(cachix, self.config.cache().unwrap(), dry_run)?;

                Some(cachix_version(cachix)?)
            }
            None => None,
        };

        if self.output_dir.is_dir() {
            log::warn!("Removing old artifact dir");
            if dry_run {
                println!("[DRYRUN] would remove old artifact dir");
            } else {
                fs::remove_dir_all(&self.output_dir)?;
            }
        }
        fs::create_dir_all(&self.output_dir)?;

        let mut summary = Summary::new(
            self.cwd.clone(),
            nix_version,
            cachix_version,
            git_revision,
            self.width,
        );

        let all_succeeded = self.build_all(dry_run, &mut summary)?;

        if all_succeeded {
            for pin in self.config.pins() {
                // TODO
            }
        }

        // TODO: json output option
        summary.print();

        Ok(all_succeeded)
    }
}
