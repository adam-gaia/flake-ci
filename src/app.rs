use crate::config::Config;
use crate::nix::{run, run_stream};
use crate::system;
use anyhow::{Error, Result};
use log::{debug, info, warn};
use std::char::MAX;
use std::collections::HashMap;
use std::fmt::Display;
use std::path::Path;
use std::path::PathBuf;
use which::which;

const INDENT: &str = "  ";
const MAX_WIDTH: usize = 80;

#[derive(Debug)]
pub enum Status {
    Skipped,
    Success,
    Fail,
}

fn register(map: &mut HashMap<String, Vec<String>>, output_name: String, job_name: String) {
    if !map.contains_key(&output_name) {
        map.insert(output_name.clone(), Vec::new());
    }
    map.get_mut(&output_name).unwrap().push(job_name);
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
        "(dirty)"
    };
    let revision = format!("{commit_hash}{dirty}");
    Ok(revision)
}

fn format_println<S: AsRef<str> + Display, T: AsRef<str> + Display>(left: S, right: T) {
    let used_space = left.as_ref().len() + right.as_ref().len();
    assert!(used_space < MAX_WIDTH, "Line too big");
    let n = MAX_WIDTH - used_space;
    let dots = ".".repeat(n);
    let line = format!("{left}{dots}{right}");
    println!("{line}");
}

#[derive(Debug)]
struct Summary {
    dry_run: bool,
    skipped_outputs: Vec<String>,
    successes: HashMap<String, Vec<String>>,
    fails: HashMap<String, Vec<String>>,
    skips: HashMap<String, Vec<String>>,
    nix_version: String,
    git_revision: String,
}

impl Summary {
    pub fn new(dry_run: bool, nix_version: String, git_revision: String) -> Self {
        Self {
            dry_run,
            skipped_outputs: Vec::new(),
            successes: HashMap::new(),
            fails: HashMap::new(),
            skips: HashMap::new(),
            nix_version,
            git_revision,
        }
    }

    pub fn skip_output(&mut self, output: &str) {
        self.skipped_outputs.push(output.to_string());
    }

    pub fn register_success(&mut self, output_name: String, job_name: String) {
        register(&mut self.successes, output_name, job_name);
    }

    pub fn register_fail(&mut self, output_name: String, job_name: String) {
        register(&mut self.fails, output_name, job_name);
    }

    pub fn register_skip(&mut self, output_name: String, job_name: String) {
        register(&mut self.skips, output_name, job_name);
    }

    pub fn print(&self) {
        let bar = "=".repeat(MAX_WIDTH);
        println!("{bar}");
        println!("Summary");

        for output in &self.skipped_outputs {
            format_println(format!("> {output}"), "skipped (does not exist in flake)");
        }

        for (output, jobs) in &self.successes {
            println!("> {output}");
            for job in jobs {
                format_println(format!("{INDENT}- {job}"), "success");
            }
        }

        let skip_note = if self.dry_run { " (dry run)" } else { "" };
        for (output, jobs) in &self.skips {
            println!("> {output}");

            for job in jobs {
                format_println(format!("{INDENT}- {job}"), format!("skipped{skip_note}"));
            }
        }

        for (output, jobs) in &self.fails {
            println!("> {output}");
            for job in jobs {
                format_println(format!("{INDENT}- {job}"), "failed");
            }
        }

        println!("Git revision: {}", self.git_revision);
        println!("Nix version: '{}'", self.nix_version);
    }
}

#[derive(Debug)]
pub struct App {
    config: Config,
    nix: PathBuf,
    system: String,
}

impl App {
    pub fn with_config(config: Config) -> Result<Self> {
        let system = system().to_string();
        let nix = which::which("nix")?;
        Ok(Self {
            config,
            nix,
            system,
        })
    }

    fn attributes(&self, ttype: &str) -> Result<Vec<String>> {
        let args = &[
            "eval",
            &format!(".#{ttype}.{}", &self.system),
            "--apply",
            "builtins.attrNames",
            "--json",
        ];
        let stdout = run(&self.nix, args)?;
        let attributes: Vec<String> = serde_json::from_str(&stdout)?;
        Ok(attributes)
    }

    fn derivation_path(&self, ttype: &str, attribute: &str) -> Result<String> {
        let args = &[
            "eval",
            &format!(".#{ttype}.{}.{attribute}", &self.system),
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
        let status = run_stream(&self.nix, args, &self.config.env, dry_run)?;
        Ok(status)
    }

    pub fn run(&self, dry_run: bool) -> Result<()> {
        let nix_version = nix_version(&self.nix)?;
        let git_revision = git_revision()?;
        let mut summary = Summary::new(dry_run, nix_version, git_revision);

        for output in &self.config.outputs {
            debug!("type: {output}");

            let Ok(attributes) = self.attributes(output) else {
                warn!("No such entry: .#{output}");
                summary.skip_output(output);
                continue;
            };

            for attribute in attributes {
                debug!("attr: {attribute}");

                let path = self.derivation_path(output, &attribute)?;
                debug!("path: {path}");

                let derivation = format!(".#{output}.{}.{attribute}", &self.system);
                info!("Building {derivation}");
                let status = self.build(&path, dry_run)?;

                info!("Done building {derivation}");
                match status {
                    Status::Skipped => {
                        summary.register_skip(output.to_string(), derivation);
                    }
                    Status::Fail => {
                        summary.register_fail(output.to_string(), derivation);
                    }
                    Status::Success => {
                        summary.register_success(output.to_string(), derivation);
                    }
                }
            }
        }

        // TODO: json output option
        summary.print();

        Ok(())
    }
}
