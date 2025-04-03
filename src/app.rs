use crate::config::{Config, System};
use crate::nix::{run, run_stream};
use anyhow::{bail, Result};
use log::{debug, error, info, warn};
use owo_colors::{OwoColorize, Style};
use std::collections::HashMap;
use std::fmt::Display;
use std::fs;
use std::ops::Deref;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;
use which::which;

const INDENT: &str = "  ";
const STATUS_PREFIX: &str = "> ";
const SUBSTATUS_PREFIX: &str = "- ";

#[derive(Debug)]
pub enum Status {
    Skipped,
    Success,
    Fail,
}

fn register<T>(map: &mut HashMap<String, Vec<T>>, output_name: String, job: T) {
    if !map.contains_key(&output_name) {
        map.insert(output_name.clone(), Vec::new());
    }
    map.get_mut(&output_name).unwrap().push(job);
}

fn nix_version(nix: &Path) -> Result<String> {
    let output = run(nix, &["--version"])?;
    let nix_version = output.lines().next().unwrap();
    Ok(nix_version.to_string())
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
    if !diff.starts_with(".") {
        diff = format!("./{}", diff);
    }
    diff
}

#[derive(Debug)]
struct Summary {
    cwd: PathBuf,
    dry_run: bool,
    skipped_outputs: Vec<String>,
    successes: HashMap<String, Vec<(String, Option<PathBuf>)>>,
    fails: HashMap<String, Vec<(String, String)>>,
    skips: HashMap<String, Vec<String>>,
    nix_version: String,
    git_revision: String,
    width: usize,
}

impl Summary {
    pub fn new(
        cwd: PathBuf,
        dry_run: bool,
        nix_version: String,
        git_revision: String,
        width: usize,
    ) -> Self {
        Self {
            cwd,
            dry_run,
            skipped_outputs: Vec::new(),
            successes: HashMap::new(),
            fails: HashMap::new(),
            skips: HashMap::new(),
            nix_version,
            git_revision,
            width,
        }
    }

    pub fn skip_output(&mut self, output: &str) {
        self.skipped_outputs.push(output.to_string());
    }

    pub fn register_success(
        &mut self,
        output_name: String,
        job_name: String,
        artifact: Option<PathBuf>,
    ) {
        register(&mut self.successes, output_name, (job_name, artifact));
    }

    pub fn register_fail(&mut self, output_name: String, job_name: String, log_command: String) {
        register(&mut self.fails, output_name, (job_name, log_command));
    }

    pub fn register_skip(&mut self, output_name: String, job_name: String) {
        register(&mut self.skips, output_name, job_name);
    }

    fn print_line(&self, left: &str, right: &str, style: Option<&Style>, extra_note: Option<&str>) {
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
                )
            }
            None => {
                println!("{left}{dots}{right}{extra_note}");
            }
        };
    }

    fn print_status_line(
        &self,
        left: &str,
        right: &str,
        style: Option<&Style>,
        extra_note: Option<&str>,
    ) {
        assert_eq!(STATUS_PREFIX.len(), INDENT.len());
        let left = format!("{STATUS_PREFIX}{left}");
        self.print_line(&left, right, style, extra_note);
    }

    fn print_substatus_line(
        &self,
        left: &str,
        right: &str,
        style: &Style,
        extra_note: Option<&str>,
    ) {
        assert_eq!(SUBSTATUS_PREFIX.len(), INDENT.len());
        let left = format!("{INDENT}{SUBSTATUS_PREFIX}{left}");
        self.print_line(&left, right, Some(style), extra_note);
    }

    fn print_substatus_attribute(name: &str, attribute: &str) {
        println!("{INDENT}{INDENT}{name}: {attribute}");
    }

    fn print_version(slug: &str, version: &str) {
        println!(
            "{slug}: {}",
            version.if_supports_color(owo_colors::Stream::Stdout, |text| text.bold())
        )
    }

    pub fn print(&self) {
        let yellow = Style::new().yellow().bold();
        let green = Style::new().green().bold();
        let red = Style::new().red().bold();

        let bar = "=".repeat(self.width);
        println!("{bar}");
        println!("Summary");

        for output in &self.skipped_outputs {
            self.print_status_line(output, "skipped", Some(&yellow), Some("(not found)"));
        }

        for (output, jobs) in &self.successes {
            self.print_status_line(output, "", None, None);
            for (job_name, artifact) in jobs {
                self.print_substatus_line(job_name, "success", &green, None);

                if let Some(artifact) = artifact {
                    let artifact = rel_to_cwd(artifact, &self.cwd);
                    Summary::print_substatus_attribute("artifact", &artifact);
                }
            }
        }

        for (output, jobs) in &self.skips {
            self.print_status_line(output, "", None, None);
            for job in jobs {
                self.print_substatus_line(job, "skipped", &yellow, Some("(dry run)"));
            }
        }

        for (output, jobs) in &self.fails {
            println!("> {output}");
            for (job, log_command) in jobs {
                self.print_substatus_line(job, "failed", &red, None);
                Summary::print_substatus_attribute("log command", &log_command);
            }
        }

        Summary::print_version("Git revision", &self.git_revision);
        Summary::print_version("Nix version:", &self.nix_version);
    }
}

#[derive(Debug)]
pub struct App {
    cwd: PathBuf,
    working_dir: PathBuf,
    output_dir: PathBuf,
    nix_result_dir: PathBuf,
    config: Config,
    nix: PathBuf,
    system: System,
    width: usize,
}

impl App {
    pub fn with_config(
        cwd: PathBuf,
        working_dir: PathBuf,
        system: System,
        width: usize,
        config: Config,
    ) -> Result<Self> {
        let output_dir = working_dir.join(config.artifact_dir());
        let nix_result_dir = working_dir.join("result");
        let nix = which::which("nix")?;
        Ok(Self {
            cwd,
            working_dir,
            output_dir,
            nix_result_dir,
            config,
            nix,
            system,
            width,
        })
    }

    fn attributes(&self, ttype: &str, system: &System) -> Result<Vec<String>> {
        let args = &[
            "eval",
            &format!(".#{ttype}.{}", system),
            "--apply",
            "builtins.attrNames",
            "--json",
        ];
        let stdout = run(&self.nix, args)?;
        let attributes: Vec<String> = serde_json::from_str(&stdout)?;
        Ok(attributes)
    }

    fn derivation_path(&self, ttype: &str, system: &System, attribute: &str) -> Result<String> {
        let args = &[
            "eval",
            &format!(".#{ttype}.{}.{attribute}", system),
            "--apply",
            "pkg: pkg.drvPath",
            "--raw",
        ];
        let path = run(&self.nix, args)?;
        Ok(path)
    }

    fn build(&self, path: &str, dry_run: bool) -> Result<Status> {
        let args = &[
            "build",
            &format!("{path}^*"),
            "--log-lines",
            "0",
            "--print-build-logs",
        ];
        let status = run_stream(&self.nix, args, self.config.env(), dry_run)?;
        Ok(status)
    }

    pub fn run(&self, dry_run: bool) -> Result<bool> {
        let mut all_succeeded = true;

        let nix_version = nix_version(&self.nix)?;
        let git_revision = git_revision()?;
        let mut summary = Summary::new(
            self.cwd.to_path_buf(),
            dry_run,
            nix_version,
            git_revision,
            self.width,
        );

        if self.output_dir.is_dir() {
            log::warn!("Removing old artifact dir");
            if dry_run {
                println!("[DRYRUN] would remove old artifact dir")
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

                let Ok(attributes) = self.attributes(output, system) else {
                    warn!("No such entry: .#{output}");
                    summary.skip_output(output);
                    continue;
                };

                for attribute in &attributes {
                    debug!("Attr: {attribute}");

                    let path = self.derivation_path(output, &system, &attribute)?;
                    debug!("Path: {path}");

                    let derivation = format!(".#{output}.{}.{attribute}", system);
                    info!("Building {derivation}");
                    let status = self.build(&path, dry_run)?;

                    info!("Done building {derivation}");
                    match status {
                        Status::Skipped => {
                            summary.register_skip(output.to_string(), derivation);
                        }
                        Status::Fail => {
                            all_succeeded = false;
                            let log_command = format!("`nix log {path}`");
                            summary.register_fail(output.to_string(), derivation, log_command);
                        }
                        Status::Success => {
                            let artifact = if !dry_run
                                && self.config.save_artifact(output, system, attribute)
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

                            summary.register_success(
                                output.to_string(),
                                derivation.clone(),
                                artifact,
                            );
                        }
                    }
                }
            }
        }

        // TODO: json output option
        summary.print();

        Ok(all_succeeded)
    }
}
