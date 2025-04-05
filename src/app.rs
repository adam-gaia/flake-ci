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

const INDENT: &str = "  ";
const STATUS_PREFIX: &str = "> ";
const SUBSTATUS_PREFIX: &str = "- ";
const CACHIX_AUTH_KEY: &str = "CACHIX_AUTH_TOKEN";
const CACHIX_SIGNING_KEY: &str = "CACHIX_SIGNING_KEY";

#[derive(Debug)]
pub enum Status {
    Skipped,
    Success,
    Fail,
}

fn register<T>(map: &mut HashMap<String, Vec<T>>, output_name: &str, job: T) {
    if !map.contains_key(output_name) {
        map.insert(output_name.to_string(), Vec::new());
    }
    map.get_mut(output_name).unwrap().push(job);
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

fn rel_to_cwd(p: &Path, cwd: &Path) -> String {
    let mut diff = pathdiff::diff_paths(p, cwd).unwrap().display().to_string();
    if !diff.starts_with('.') {
        diff = format!("./{diff}");
    }
    diff
}

#[derive(Debug)]
struct Summary {
    cwd: PathBuf,
    skipped_outputs: Vec<String>,
    successes: HashMap<String, Vec<(String, Option<PathBuf>)>>,
    fails: HashMap<String, Vec<(String, String)>>,
    skips: HashMap<String, Vec<String>>,
    nix_version: String,
    cachix_version: Option<String>,
    git_revision: String,
    width: usize,
}

impl Summary {
    pub fn new(
        cwd: PathBuf,
        nix_version: String,
        cachix_version: Option<String>,
        git_revision: String,
        width: usize,
    ) -> Self {
        Self {
            cwd,
            skipped_outputs: Vec::new(),
            successes: HashMap::new(),
            fails: HashMap::new(),
            skips: HashMap::new(),
            nix_version,
            git_revision,
            cachix_version,
            width,
        }
    }

    pub fn skip_output(&mut self, output: &str) {
        self.skipped_outputs.push(output.to_string());
    }

    pub fn register_success(
        &mut self,
        output_name: &str,
        job_name: String,
        artifact: Option<PathBuf>,
    ) {
        register(&mut self.successes, output_name, (job_name, artifact));
    }

    pub fn register_fail(&mut self, output_name: &str, job_name: String, log_command: String) {
        register(&mut self.fails, output_name, (job_name, log_command));
    }

    pub fn register_skip(&mut self, output_name: &str, job_name: String) {
        register(&mut self.skips, output_name, job_name);
    }

    fn print_line(left: &str, right: &str, style: Option<&Style>, extra_note: Option<&str>) {
        let extra_note = match extra_note {
            Some(note) => &format!(" {note}"),
            None => "",
        };

        let used_space = left.len() + right.len() + extra_note.len();
        assert!(used_space < super::MAX_WIDTH, "Line too big");

        let dots = if right.is_empty() {
            String::new()
        } else {
            let n = super::MAX_WIDTH - used_space;
            ".".repeat(n)
        };

        match style {
            Some(style) => {
                println!(
                    "{left}{dots}{}{extra_note}",
                    right.if_supports_color(owo_colors::Stream::Stdout, |text| text.style(*style)),
                );
            }
            None => {
                println!("{left}{dots}{right}{extra_note}");
            }
        };
    }

    fn print_status_line(left: &str, right: &str, style: Option<&Style>, extra_note: Option<&str>) {
        assert_eq!(STATUS_PREFIX.len(), INDENT.len());
        let left = format!("{STATUS_PREFIX}{left}");
        Summary::print_line(&left, right, style, extra_note);
    }

    fn print_substatus_line(left: &str, right: &str, style: &Style, extra_note: Option<&str>) {
        assert_eq!(SUBSTATUS_PREFIX.len(), INDENT.len());
        let left = format!("{INDENT}{SUBSTATUS_PREFIX}{left}");
        Summary::print_line(&left, right, Some(style), extra_note);
    }

    fn print_substatus_attribute(name: &str, attribute: &str) {
        println!("{INDENT}{INDENT}{name}: {attribute}");
    }

    fn print_version(slug: &str, version: &str) {
        println!(
            "{slug}: {}",
            version.if_supports_color(owo_colors::Stream::Stdout, |text| text.bold())
        );
    }

    pub fn print(&self) {
        let yellow = Style::new().yellow().bold();
        let green = Style::new().green().bold();
        let red = Style::new().red().bold();

        let bar = "=".repeat(self.width);
        println!("{bar}");
        println!("Summary");

        for output in &self.skipped_outputs {
            Summary::print_status_line(output, "skipped", Some(&yellow), Some("(not found)"));
        }

        for (output, jobs) in &self.successes {
            Summary::print_status_line(output, "", None, None);
            for (job_name, artifact) in jobs {
                Summary::print_substatus_line(job_name, "success", &green, None);

                if let Some(artifact) = artifact {
                    let artifact = rel_to_cwd(artifact, &self.cwd);
                    Summary::print_substatus_attribute("artifact", &artifact);
                }
            }
        }

        for (output, jobs) in &self.skips {
            Summary::print_status_line(output, "", None, None);
            for job in jobs {
                Summary::print_substatus_line(job, "skipped", &yellow, Some("(dry run)"));
            }
        }

        for (output, jobs) in &self.fails {
            println!("> {output}");
            for (job, log_command) in jobs {
                Summary::print_substatus_line(job, "failed", &red, None);
                Summary::print_substatus_attribute("log command", log_command);
            }
        }

        Summary::print_version("Git revision", &self.git_revision);
        Summary::print_version("Nix version:", &self.nix_version);
        if let Some(cachix_version) = &self.cachix_version {
            Summary::print_version("Cachix version", cachix_version);
        };
    }
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

    pub fn run(&self, dry_run: bool) -> Result<bool> {
        let mut all_succeeded = true;

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

        let mut summary = Summary::new(
            self.cwd.clone(),
            nix_version,
            cachix_version,
            git_revision,
            self.width,
        );

        if self.output_dir.is_dir() {
            log::warn!("Removing old artifact dir");
            if dry_run {
                println!("[DRYRUN] would remove old artifact dir");
            } else {
                fs::remove_dir_all(&self.output_dir)?;
            }
        }
        fs::create_dir_all(&self.output_dir)?;

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
