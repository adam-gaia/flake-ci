use super::Derivation;
use crate::app::{get_type_of_check, parse_check_name};
use crate::config::Config;
use crate::graph::Graph;
use crate::model::{NamePattern, SymbolicOutput, System, SystemPattern};
use crate::nix::run;
use anyhow::{bail, Result};
use log::warn;
use owo_colors::OwoColorize;
use std::cmp::Eq;
use std::fmt::{Debug, Display};
use std::ops::IndexMut;
use std::path::{Path, PathBuf};
use std::{collections::HashMap, hash::Hash};

fn derivations(nix: &Path, output_name: String, system: System) -> Result<Vec<Derivation>> {
    let args = &[
        "eval",
        &format!(".#{output_name}.{system}"),
        "--apply",
        "builtins.attrNames",
        "--json",
    ];
    let stdout = run(nix, args)?;
    let names: Vec<String> = serde_json::from_str(&stdout)?;
    let drvs = names
        .iter()
        .map(|name| Derivation::new(output_name.clone(), system, name.to_owned()))
        .collect();
    Ok(drvs)
}

#[derive(Debug)]
enum SystemValue {
    Skipped,
    Discovered(Vec<Derivation>),
}

#[derive(Debug)]
pub struct DrvTree {
    config: Config,
    systems: Vec<System>,
    outputs: HashMap<String, HashMap<System, SystemValue>>,
}

impl DrvTree {
    pub fn new(nix: &Path, config: Config, current_system: System) -> Result<Self> {
        let systems = config.systems();
        let output_names = config.build_outputs();

        let mut outputs = HashMap::new();
        for output_name in output_names {
            let mut systems_map = HashMap::new();

            for system in &systems {
                if *system != current_system {
                    // TODO: cross compile??
                    warn!("Skipping system {system}");
                    systems_map.insert(system.clone(), SystemValue::Skipped);
                    continue;
                }

                let drvs = derivations(nix, output_name.to_owned(), *system)?;
                systems_map.insert(system.clone(), SystemValue::Discovered(drvs));
            }

            outputs.insert(output_name.to_string(), systems_map);
        }

        Ok(Self {
            config,
            systems,
            outputs,
        })
    }

    pub fn output_names<'a>(&'a self) -> Vec<&'a String> {
        self.outputs.keys().into_iter().collect()
    }

    pub fn systems(&self) -> &[System] {
        &self.systems
    }

    pub fn derivations(&self) -> Vec<&Derivation> {
        let mut derivations = Vec::new();
        for system_map in self.outputs.values() {
            for system_value in system_map.values() {
                match system_value {
                    SystemValue::Skipped => continue,
                    SystemValue::Discovered(drvs) => {
                        for drv in drvs {
                            derivations.push(drv);
                        }
                    }
                }
            }
        }
        derivations
    }

    pub fn matrix_from_symbolic(&self, path: &SymbolicOutput) -> Result<Vec<Derivation>> {
        let all_outputs = self.output_names();
        let all_systems = self.systems();
        let all_drvs = self.derivations();
        let drvs = path.matrix(
            all_outputs.as_ref(),
            all_systems.as_ref(),
            all_drvs.as_ref(),
        )?;
        Ok(drvs)
    }

    pub fn matching(&self, path: &SymbolicOutput) -> Result<Vec<Derivation>> {
        self.matrix_from_symbolic(path)
    }

    pub fn build_order(&self) -> Result<Vec<Vec<Derivation>>> {
        let mut graph = Graph::new();

        for drv in self.derivations() {
            graph.add_node((*drv).clone());
        }

        // If the checks check a nammed output, mark the output as a dependency of the check
        for system in &self.systems {
            let checks = self.matching(&SymbolicOutput::new(
                NamePattern::Specified(String::from("checks")),
                SystemPattern::Specified(*system),
                NamePattern::Any,
            ))?;

            for check in &checks {
                let (check_type, name) = parse_check_name(check)?;

                let possible_child = self.matching(&SymbolicOutput::new(
                    NamePattern::Specified(check_type.to_string()),
                    SystemPattern::Specified(*system),
                    NamePattern::Specified(name.to_string()),
                ))?;

                if let Some(child) = possible_child.first() {
                    graph.mark_dep(&check, &child)?;
                }
            }
        }

        // Add in extra dependencies found in config file
        for (child_symbolic, prereqs_symbolic) in self.config.output_configs() {
            for child in self.matrix_from_symbolic(&child_symbolic)? {
                for prereq_symbolic in prereqs_symbolic {
                    for prereq in self.matrix_from_symbolic(&prereq_symbolic)? {
                        graph.mark_dep(&prereq, &child)?;
                    }
                }
            }
        }

        let walker = graph.walker();
        let chains = walker.chains();

        // TODO: par resolve all chains at the same time?

        Ok(chains)
    }
}
