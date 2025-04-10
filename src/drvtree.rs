use crate::config::Config;
use crate::graph::Graph;
use crate::model::{BuildStatus, Derivation, Status};
use crate::model::{NamePattern, SymbolicOutput, System, SystemPattern};
use crate::nix::{run, run_stream};
use crate::util::get_type_of_check;
use crate::util::parse_check_name;
use crate::{CachixSettings, Settings};
use anyhow::{bail, Result};
use log::{debug, info, warn};
use owo_colors::OwoColorize;
use std::cmp::Eq;
use std::collections::{HashSet, VecDeque};
use std::fmt::{Debug, Display};
use std::ops::IndexMut;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::thread::{self, Thread};
use std::time::Duration;
use std::{collections::HashMap, hash::Hash};
use winnow::stream::StreamIsPartial;

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
pub struct App {
    settings: Settings,
    systems: Vec<System>,
    outputs: HashMap<String, HashMap<System, SystemValue>>,
}

impl App {
    pub fn with_settings(settings: Settings) -> Result<Self> {
        let repo_config = settings.repo_config();
        let systems: Vec<System> = repo_config
            .systems()
            .iter()
            .filter(|system| **system == settings.current_system()) // TODO: remove filter for cross compiling eventually?
            .map(|x| x.clone())
            .collect();
        let output_names = repo_config.build_outputs();

        let mut outputs = HashMap::new();
        for output_name in output_names {
            let mut systems_map = HashMap::new();

            for system in &systems {
                let Ok(drvs) = derivations(settings.nix(), output_name.to_owned(), *system) else {
                    warn!("Flake does not contain outputs under '{output_name}.{system}'");
                    continue;
                };

                systems_map.insert(system.clone(), SystemValue::Discovered(drvs));
            }

            outputs.insert(output_name.to_string(), systems_map);
        }

        Ok(Self {
            settings,
            systems,
            outputs,
        })
    }

    fn output_names<'a>(&'a self) -> Vec<&'a String> {
        self.outputs.keys().into_iter().collect()
    }

    fn systems(&self) -> &[System] {
        &self.systems
    }

    fn derivations(&self) -> Vec<&Derivation> {
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

    fn matrix_from_symbolic(&self, path: &SymbolicOutput) -> Result<Vec<Derivation>> {
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

    fn matching(&self, path: &SymbolicOutput) -> Result<Vec<Derivation>> {
        self.matrix_from_symbolic(path)
    }

    fn build_graph(&self) -> Result<Graph<Derivation>> {
        let mut graph = Graph::new();

        for drv in self.derivations() {
            info!("adding drv {drv}");
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
                let Ok((check_type, name)) = parse_check_name(check) else {
                    debug!("Not a standard check: {check}");
                    continue;
                };

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
        for (child_symbolic, prereqs_symbolic) in self.settings.repo_config().output_configs() {
            for child in self.matrix_from_symbolic(&child_symbolic)? {
                for prereq_symbolic in prereqs_symbolic {
                    for prereq in self.matrix_from_symbolic(&prereq_symbolic)? {
                        if prereq.system() == child.system() {
                            graph.mark_dep(&prereq, &child)?;
                        }
                    }
                }
            }
        }

        debug!("Graph:\n{graph}");
        Ok(graph)
    }

    fn build_paths(&self, graph: Graph<Derivation>) -> Result<Vec<Vec<Vec<Derivation>>>> {
        let paths = graph.build_paths()?;
        Ok(paths)
    }

    fn print_build_paths(&self, paths: &[Vec<Vec<Derivation>>]) {
        for paths in paths {
            let longest_len = paths.iter().fold(0, |acc, elem| {
                let len = elem.len();
                if len > acc {
                    len
                } else {
                    acc
                }
            });
            assert!(longest_len > 0);

            let target = &paths[0][0];
            println!("Target: {target}");

            for path in paths {
                assert!(!paths.is_empty(), "Error, empty build path");

                let len = path.len();

                print!("  Dependency chain: ");
                for (i, dep) in path.iter().rev().enumerate() {
                    print!("{dep}");
                    if i < (len - 1) {
                        print!(" -> ",)
                    }
                }
                println!();
            }
        }
    }

    pub fn run(&self) -> Result<bool> {
        //let paths = self.build_paths()?;

        /*
        if self.settings.print_build_order() {
            self.print_build_paths(&paths);
            std::process::exit(0);
        }
        */

        let graph = self.build_graph()?;
        let dependency_list = graph.dependency_list();
        let queue = Queue::new(dependency_list);

        let statuses = Arc::new(Mutex::new(HashMap::new()));
        let completed = Arc::new(AtomicUsize::new(0));
        let queue = Arc::new(Mutex::new(queue));

        let num_threads = self.settings.num_threads();
        debug!("Using {num_threads} threads");

        for _ in 0..num_threads {
            let queue = Arc::clone(&queue);
            let completed = Arc::clone(&completed);
            let statuses = Arc::clone(&statuses);

            let nix = self.settings.nix.clone();
            let dry_run = self.settings.dry_run.clone();
            let cachix_settings = self.settings.cachix_settings.clone();

            thread::spawn(move || loop {
                let task = {
                    let mut queue = queue.lock().unwrap();
                    queue.next()
                };

                let Some(task) = task else {
                    let done = {
                        let queue = queue.lock().unwrap();
                        queue.is_done()
                    };
                    if done {
                        // All done
                        break;
                    }

                    // Otherwise, wait for a new task to become unblocked
                    thread::sleep(Duration::from_millis(10)); // TODO: configure
                    continue;
                };

                let status = build(&nix, &cachix_settings, dry_run, &task).unwrap();
                {
                    let mut queue = queue.lock().unwrap();

                    let mut statuses = statuses.lock().unwrap();

                    if status == BuildStatus::Failed {
                        // Mark the dependents of this task as skipped
                        let dependents = queue.complete_failed(&task);
                        for dep in dependents {
                            statuses.insert(dep, Status::Skipped);
                        }
                    } else {
                        queue.complete_success(&task);
                    }
                }

                {
                    let mut statuses = statuses.lock().unwrap();
                    statuses.insert(task, status.to_output_status());
                }

                // Do this last because it can unblock our main thread
                completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            });
        }

        // Wait for tasks to finish
        loop {
            let done = {
                let queue = queue.lock().unwrap();
                queue.is_done()
            };
            if done {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        thread::sleep(Duration::from_millis(100));

        let statuses = Arc::try_unwrap(statuses)
            .expect("cannot un-arc")
            .into_inner()
            .expect("mutex poisoned");

        for (task, status) in &statuses {
            println!("{task}: {status}");
        }

        Ok(statuses.iter().all(|(_, s)| *s != Status::Failed))
    }
}

#[derive(Debug)]
struct Queue {
    queue: VecDeque<Derivation>,
    dependency_list: HashMap<Derivation, HashSet<Derivation>>,
}

impl Queue {
    pub fn new(dependency_list: HashMap<Derivation, HashSet<Derivation>>) -> Self {
        let mut queue = VecDeque::new();

        // Seed the queue with any unblocked tasks
        let mut initial_tasks: HashSet<Derivation> =
            dependency_list.keys().map(|x| x.clone()).collect();

        // If the task is blocked by anything, it can't be added to the initial set
        for (_blocker, blockees) in &dependency_list {
            for blocked in blockees {
                initial_tasks.remove(&blocked);
            }
        }

        for task in initial_tasks {
            queue.push_front(task);
        }

        Self {
            queue,
            dependency_list,
        }
    }

    /// Get the next task from the front of the queue
    pub fn next(&mut self) -> Option<Derivation> {
        self.queue.pop_front()
    }

    pub fn recurse_remove(&mut self, parent: &Derivation) -> HashSet<Derivation> {
        let mut foo = HashSet::new();
        if let Some(dependents) = self.dependency_list.remove(parent) {
            for dep in dependents {
                foo.insert(dep.clone());
                foo.union(&self.recurse_remove(&dep));
            }
        }
        foo
    }

    /// Remove this task from the dependency list
    /// Do not add its dependents to the queue, instead return them.
    /// Also handles dependents of those tasks, and so on
    pub fn complete_failed(&mut self, task: &Derivation) -> HashSet<Derivation> {
        self.recurse_remove(task)
    }

    /// Mark a task as complete, removing it from the internal dependency list.
    /// If the task blocked other tasks, those tasks are enqueued
    pub fn complete_success(&mut self, task: &Derivation) {
        if let Some(dependees) = self.dependency_list.remove(task) {
            for dep in dependees {
                self.queue.push_back(dep);
            }
        }
    }

    pub fn is_done(&self) -> bool {
        self.dependency_list.is_empty()
    }
}

fn build(
    nix_path: &Path,
    cachix_settings: &Option<CachixSettings>,
    dry_run: bool,
    derivation: &Derivation,
) -> Result<BuildStatus> {
    info!("building {derivation}");

    let nix_args = &[
        "build",
        &format!("{derivation}^*"),
        "--log-lines",
        "0",
        "--print-build-logs",
        "--print-out-paths",
    ];

    if let Some(cachix_settings) = cachix_settings {
        if cachix_settings.publish() {
            let cachix_path = cachix_settings.cachix_path();
            let cache = cachix_settings.cache_name();

            // Run nix build under cachix. Cachix will push all built paths
            // TODO: make 'cachix watch-exec nix ...' a function
            let nix = nix_path.display().to_string();
            let mut args = vec!["watch-exec", &cache, "--", &nix];
            args.extend_from_slice(nix_args);
            let status = run_stream(&cachix_path, &args, None, dry_run)?;
            return Ok(status);
        }
    }
    let status = run_stream(&nix_path, nix_args, None, dry_run)?;
    Ok(status)
}
