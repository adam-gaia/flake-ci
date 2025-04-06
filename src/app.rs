use crate::config::{Config, ParseError, System};
use crate::graph::Graph;
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
use summary::Summary;
use winnow::prelude::*;

const CACHIX_AUTH_KEY: &str = "CACHIX_AUTH_TOKEN";
const CACHIX_SIGNING_KEY: &str = "CACHIX_SIGNING_KEY";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Derivation {
    output: String,
    system: System,
    name: String,
}

impl Derivation {
    pub fn new(output: String, system: System, name: String) -> Self {
        Self {
            output,
            system,
            name,
        }
    }
}

fn derivation(s: &mut &str) -> winnow::Result<Derivation> {
    winnow::combinator::seq! {Derivation {
        output: crate::config::name,
        _: ".",
        system:  crate::config::system,
        _: ".",
        name:  crate::config::name,
    }}
    .parse_next(s)
}

impl FromStr for Derivation {
    type Err = ParseError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        derivation.parse(s).map_err(|e| ParseError::from_parse(&e))
    }
}

impl Display for Derivation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, ".#{}.{}.{}", self.output, self.system, self.name)
    }
}

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

fn find_check_type(input: &str) -> Result<&'static str> {
    let mut input = input.to_lowercase();
    if let Some(stripped) = input.strip_suffix('s') {
        input = stripped.to_string();
    };

    let res = match input.as_str() {
        "pkg" | "package" => "packages",
        "devshell" | "shell" => "devShells",
        "nixo" | "nixosconfig" | "nixosconfiguration" => "nixosConfigurations",
        "darwin" | "darwinconfig" | "darwinconfiguration" => "darwinConfigurations",
        "home" | "homeconfig" | "homeconfiguration" => "homeConfigurations",
        "system" | "systemconfig" | "systemconfiguration" => "systemConfigs",
        _ => bail!("Unknown check type"),
    };

    Ok(res)
}

fn get_type_of_check(derivation: &Derivation) -> Result<&'static str> {
    let name = &derivation.name;
    let Some((prefix, _)) = name.split_once('-') else {
        bail!("TODO: better error message");
    };

    find_check_type(&prefix)
}

fn check_checks_derivation(check: &Derivation, drv: &Derivation) -> bool {
    if check.system == drv.system {
        if let Some((prefix, suffix)) = check.name.split_once('-') {
            if let Ok(check_type) = find_check_type(prefix) {
                if check_type.to_lowercase() == drv.output.to_lowercase() {
                    return suffix.to_lowercase() == drv.name.to_lowercase();
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

    fn derivation_path(&self, derivation: &Derivation) -> Result<String> {
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
                // TODO: cross compiling?? Will probably also need to fix the graph stuff
                warn!("Skipping system {}", system);
                continue;
            }

            // TODO: build graph so that packages aren't built unless checks pass

            let mut sets = HashMap::new();
            let mut graph: Graph<(Derivation, String)> = Graph::new();
            for output in self.config.build_outputs() {
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

            let walker = graph.walker();
            let chains = walker.chains();

            for chain in &chains {
                debug!("chain: {chain:?}");
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

                    let output = &derivation.output;
                    let attribute = &derivation.name;

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
                                let output = &derivation.output;
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
