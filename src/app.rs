use crate::config::Config;
use crate::nix::{run, run_stream};
use crate::system;
use anyhow::{Error, Result};
use log::{debug, info, warn};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use which::which;

const INDENT: &str = "  ";
const DOTS: &str = "....................";

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
    let output = run(&git, &["rev-parse", "--short", "HEAD"])?;
    Ok(output)
}

#[derive(Debug)]
struct Summary {
    skipped_outputs: Vec<String>,
    successes: HashMap<String, Vec<String>>,
    fails: HashMap<String, Vec<String>>,
    skips: HashMap<String, Vec<String>>,
    nix_version: String,
    git_revision: String,
}

impl Summary {
    pub fn new(nix_version: String, git_revision: String) -> Self {
        Self {
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
        println!("===========================");
        println!("Summary");

        for output in &self.skipped_outputs {
            println!("> {output}{DOTS}skipped (does not exist in flake)")
        }

        for (output, jobs) in &self.successes {
            println!("> {output}");
            for job in jobs {
                println!("{INDENT}- {job}{DOTS}success")
            }
        }

        for (output, jobs) in &self.skips {
            println!("> {output}");
            for job in jobs {
                println!("{INDENT}- {job}{DOTS}skipped")
            }
        }

        for (output, jobs) in &self.fails {
            println!("> {output}");
            for job in jobs {
                println!("{INDENT}- {job}{DOTS}success")
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
        let mut summary = Summary::new(nix_version, git_revision);

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
