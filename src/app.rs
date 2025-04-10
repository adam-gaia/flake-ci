use crate::config::Config;
use crate::graph::Graph;
use crate::model::Status;
use crate::model::{NamePattern, SymbolicOutput, System, SystemPattern};
use crate::nix::{run, run_stream};
use anyhow::{bail, Result};
use log::{debug, info, warn};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt::Display;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use which::which;
mod summary;
use crate::model::Derivation;
use summary::Summary;
use winnow::prelude::*;

const CACHIX_AUTH_KEY: &str = "CACHIX_AUTH_TOKEN";
const CACHIX_SIGNING_KEY: &str = "CACHIX_SIGNING_KEY";

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

fn check_checks_derivation(check: &Derivation, drv: &Derivation) -> bool {
    if check.system() == drv.system() {
        if let Some((prefix, suffix)) = check.name().split_once('-') {
            if let Ok(check_type) = find_check_type(prefix) {
                if check_type.to_lowercase() == drv.output().to_lowercase() {
                    return suffix.to_lowercase() == drv.name().to_lowercase();
                }
            }
        }
    }

    false
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
    no_cachix: bool,
    print_build_chains: bool,
}

impl App {
    pub fn with_config(
        cwd: PathBuf,
        working_dir: &Path,
        system: System,
        width: usize,
        config: Config,
        no_cachix: bool,
        print_build_chains: bool,
    ) -> Result<Self> {
        let output_dir = working_dir.join(config.artifact_dir());
        let nix_result_dir = working_dir.join("result");

        let cachix = if no_cachix {
            None
        } else {
            match config.cache() {
                Some(_) => {
                    let Ok(cachix) = which::which("cachix") else {
                        bail!("Unable to find cachix on the $PATH (config has cachix set)");
                    };
                    Some(cachix)
                }
                None => None,
            }
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
            no_cachix,

            print_build_chains,
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

    fn derivation_path(&self, derivation: &Derivation) -> Result<String> {
        // TODO: cache this (but check if flake.nix hasn't changed) because it can take a second to run
        let args = &[
            "eval",
            &derivation.to_string(),
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

        let status = if !self.no_cachix && self.config.publish() {
            // Run nix build under cachix. Cachix will push all built paths
            // TODO: make 'cachix watch-exec nix ...' a function
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

        /*
        let all_outputs = self.config.build_outputs();
        let all_systems = &self.config.systems();
        let mut all_derivations = Vec::new(); // TODO: build up


        let mut graph: Graph<(Derivation, String)> = Graph::new();
        for system in all_systems {
            if system != &self.system {
                // TODO: cross compiling?? Will probably also need to fix the graph stuff
                warn!("Skipping system {}", system);
                continue;
            }

            let mut sets = HashMap::new();

            for output in all_outputs {
                sets.insert(output.to_owned(), HashSet::new());

                let Ok(attributes) = self.attributes(output, *system) else {
                    warn!("No such entry: .#{output}");
                    summary.skip_output(output);
                    continue;
                };

                for attribute in &attributes {
                    debug!("Attr: {attribute}");

                    let derivation =
                        Derivation::new(output.to_owned(), *system, attribute.to_owned());

                    all_derivations.push(derivation.clone());

                    let path = self.derivation_path(&derivation)?;
                    debug!("Path: {path}");

                    let similar_set = sets.get_mut(output).unwrap();
                    similar_set.insert((derivation.clone(), path.clone()));

                    graph.add_node((derivation, path));
                }
            }

            // If there are checks, mark the things they check as dependencies of the check
            if let Some(checks) = sets.remove(&String::from("checks")) {
                for (check, check_path) in checks {
                    // TODO: config should have a way to mark what output(s?) a check checks
                    let Ok(type_of_check) = get_type_of_check(&check) else {
                        warn!("Check '{check}' is not a pre-rec for building any packages");
                        continue;
                    };

                    if let Some(derivations) = sets.get(type_of_check) {
                        for (derivation, path) in derivations {
                            if check_checks_derivation(&check, derivation) {
                                graph.mark_dep(
                                    &(check.clone(), check_path.clone()),
                                    &(derivation.to_owned(), path.to_owned()),
                                )?;
                            }
                        }
                    }
                }
            };
        }

        // Add in extra dependencies from config file
        for (child, prereqs) in self.config.output_configs() {
            for child in &child.matrix(all_outputs, all_systems, &all_derivations)? {
                for parent in prereqs {
                    for parent in &parent.matrix(all_outputs, all_systems, &all_derivations)? {
                        graph.mark_dep(
                            &(parent.clone(), String::from("TODO: parent")),
                            &(child.clone(), String::from("TODO: child")),
                        )?;
                    }
                }
            }
        }

        let walker = graph.walker();
        let chains = walker.chains();

        if self.print_build_chains {
            for chain in &chains {
                let num = chain.len();
                for (i, (drv, _)) in chain.iter().enumerate() {
                    print!("{drv}");

                    if i < (num - 1) {
                        print!(" -> ");
                    }
                }
                println!();
            }
            std::process::exit(0);
        }

        let mut have_ran = HashSet::new();

        for chain in chains {
            let num_items = chain.len();
            for i in 0..num_items {
                let (derivation, path) = &chain[i];

                if have_ran.contains(derivation) {
                    continue;
                }

                info!("Building {derivation}");
                let status = self.build(&path, dry_run)?;
                info!("Done building {derivation}");

                let output = &derivation.output();
                let attribute = &derivation.name();

                match status {
                    Status::Skipped => {
                        summary.register_skip(output, derivation.to_string());
                    }
                    Status::Fail => {
                        all_succeeded = false;
                        let log_command = format!("`nix log {path}`");
                        summary.register_fail(output, derivation.to_string(), log_command);

                        let pre_rec = derivation;

                        // Mark the rest of the chain as blocked because requirement failed
                        for j in i..num_items {
                            let (derivation, _) = &chain[j];
                            if have_ran.contains(derivation) {
                                continue;
                            }
                            let output = &derivation.output();
                            summary.register_blocked(
                                output,
                                derivation.to_string(),
                                pre_rec.to_string(),
                            );
                            have_ran.insert(derivation.clone());
                        }
                        break;
                    }
                    Status::Success => {
                        let artifact =
                            if !dry_run && self.config.save_artifact(output, *system, attribute) {
                                debug!("Saving artifacts from {}", &derivation);
                                let artifact = &self.nix_result_dir;
                                if !artifact.is_symlink() {
                                    bail!("Error: todo better error message");
                                }

                                let artifact = fs::canonicalize(artifact)?;
                                debug!("artifact to save: {}", artifact.display());

                                let link = self.output_dir.join(&derivation.to_string());
                                debug!("link: {}", link.display());
                                symlink(&artifact, &link)?;

                                Some(link)
                            } else {
                                None
                            };

                        summary.register_success(output, derivation.to_string(), artifact);
                    }
                }
                have_ran.insert(derivation.clone());
            }
        }
        */

        Ok(all_succeeded)
    }

    pub fn run(&self, dry_run: bool, no_fmt: bool) -> Result<bool> {
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

        let format_result = if !no_fmt {
            run_stream(&self.nix, &["fmt"], None, dry_run)?
        } else {
            warn!("Skipping running formatter");
            Status::Skipped
        };

        let mut summary = Summary::new(
            self.cwd.clone(),
            nix_version,
            cachix_version,
            git_revision,
            self.width,
            format_result.clone(),
        );

        let all_builds_succeeded = self.build_all(dry_run, &mut summary)?;

        if all_builds_succeeded {
            for pin in self.config.pins() {
                // TODO
            }
        }

        // TODO: json output option
        summary.print();

        let all_succeeded = all_builds_succeeded && format_result != Status::Fail;
        Ok(all_succeeded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::{assert_eq, assert_ne};
    use rstest::rstest;

    fn mk_check(prefix: &str, name: &str) -> Derivation {
        let input = format!("checks.x86_64-linux.{prefix}-{name}");
        Derivation::from_str(&input).unwrap()
    }

    fn mk_plural(input: &str) -> String {
        let mut input = input.to_string();
        if input.ends_with('s') {
            return input;
        }
        input.push('s');
        input
    }

    #[rstest]
    #[case("pkg", "packages")]
    #[case("package", "packages")]
    #[case("shell", "devShells")]
    #[case("devshell", "devShells")]
    #[case("devShell", "devShells")]
    #[case("nixos", "nixosConfigurations")]
    #[case("NixOS", "nixosConfigurations")]
    #[case("nixosConfiguration", "nixosConfigurations")]
    #[case("darwin", "darwinConfigurations")]
    #[case("Darwin", "darwinConfigurations")]
    #[case("darwinConfiguration", "darwinConfigurations")]
    #[case("home", "homeConfigurations")]
    #[case("homeConfiguration", "homeConfigurations")]
    #[case("system", "systemConfigs")]
    #[case("systemConfig", "systemConfigs")]
    #[case("systemConfiguration", "systemConfigs")]
    fn test_get_type_of_check(#[case] prefix: &str, #[case] expected: &str) {
        let name = "foo";
        let drv = mk_check(prefix, name);
        // Check that match works as-is
        let actual = get_type_of_check(&drv).unwrap();
        assert_eq!(expected, actual);

        // Check that match works when plural
        let name = mk_plural(prefix);
        let drv = mk_check(&prefix, &name);
        let actual = get_type_of_check(&drv).unwrap();
        assert_eq!(expected, actual)
    }

    #[test]
    fn test_check_does_check_thing() {
        let prefix = "pkgs";
        let name = "foo";
        let check = mk_check(prefix, name);
        let drv = Derivation::new("packages".to_owned(), System::x86_linux(), name.to_owned());
        assert!(check_checks_derivation(&check, &drv));
    }

    #[test]
    fn test_check_doesnt_check_thing() {
        let prefix = "pkgs";
        let name = "foo";
        let check = mk_check(prefix, name);
        let drv = Derivation::new(
            "nixosConfigurations".to_owned(),
            System::x86_linux(),
            name.to_owned(),
        );
        //assert_eq!(check, drv);
        assert!(!check_checks_derivation(&check, &drv));
    }
}
