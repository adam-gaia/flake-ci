use crate::config::{Config, System};
use crate::nix::{run, run_stream};
use crate::system;
use anyhow::{bail, Result};
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::fmt::Display;
use std::fs;
use std::os::unix::fs::symlink;
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

fn rel_to_cwd(p: &Path, cwd: &Path) -> String {
    let cwd = cwd.display().to_string();
    p.display().to_string().replace(&cwd, ".")
}

#[derive(Debug)]
struct Summary {
    cwd: PathBuf,
    dry_run: bool,
    skipped_outputs: Vec<String>,
    successes: HashMap<String, Vec<(String, Option<PathBuf>)>>,
    fails: HashMap<String, Vec<String>>,
    skips: HashMap<String, Vec<String>>,
    nix_version: String,
    git_revision: String,
}

impl Summary {
    pub fn new(cwd: PathBuf, dry_run: bool, nix_version: String, git_revision: String) -> Self {
        Self {
            cwd,
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

    pub fn register_success(
        &mut self,
        output_name: String,
        job_name: String,
        artifact: Option<PathBuf>,
    ) {
        register(&mut self.successes, output_name, (job_name, artifact));
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
                let job_name = &job.0;
                format_println(format!("{INDENT}- {job_name}"), "success");
                if let Some(artifact) = &job.1 {
                    let artifact = rel_to_cwd(artifact, &self.cwd);
                    println!("{INDENT}{INDENT}artifact: {artifact}");
                }
            }
        }

        let mut skip_note = String::from("skipped");
        if self.dry_run {
            skip_note.push_str(" (dry run)")
        }
        for (output, jobs) in &self.skips {
            println!("> {output}");

            for job in jobs {
                format_println(format!("{INDENT}- {job}"), &skip_note);
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
    cwd: PathBuf,
    output_dir: PathBuf,
    nix_result_dir: PathBuf,
    config: Config,
    nix: PathBuf,
    system: System,
}

impl App {
    pub fn with_config(cwd: PathBuf, config: Config) -> Result<Self> {
        let output_dir = cwd.join(config.artifact_dir());
        let nix_result_dir = cwd.join("result");
        let nix = which::which("nix")?;
        let system = system()?;
        Ok(Self {
            cwd,
            output_dir,
            nix_result_dir,
            config,
            nix,
            system,
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

    pub fn run(&self, dry_run: bool) -> Result<()> {
        let nix_version = nix_version(&self.nix)?;
        let git_revision = git_revision()?;
        let mut summary = Summary::new(self.cwd.to_path_buf(), dry_run, nix_version, git_revision);

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
                            summary.register_fail(output.to_string(), derivation);
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

        Ok(())
    }
}
