use crate::model::Derivation;
use crate::model::{Arch, NamePattern, SymbolicOutput, System, SystemPattern, OS};
use anyhow::bail;
use anyhow::Result;
use s_string::s;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use serde_with::DisplayFromStr;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Display;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use winnow::prelude::*;
use winnow::stream::AsChar;

fn default_artifact_dir() -> String {
    s!("dist")
}

fn default_outputs() -> Vec<String> {
    vec![
        s!("checks"),
        s!("packages"),
        s!("devShells"),
        s!("homeConfigurations"),
        s!("darwinConfigurations"),
        s!("nixosConfigurations"),
        s!("defaultPackage"),
        s!("devShell"),
    ]
}

fn default_publish() -> bool {
    false
}

#[derive(Debug, Serialize, Deserialize)]
pub struct General {
    #[serde(rename = "output-dir", default = "default_artifact_dir")]
    pub artifact_dir: String,
}

impl Default for General {
    fn default() -> Self {
        Self {
            artifact_dir: default_artifact_dir(),
        }
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct Build {
    #[serde(default = "default_outputs")]
    outputs: Vec<String>,

    #[serde_as(as = "Vec<DisplayFromStr>")]
    #[serde(default)]
    artifacts: Vec<SymbolicOutput>,

    #[serde_as(as = "Vec<DisplayFromStr>")]
    #[serde(default)]
    architectures: Vec<Arch>,

    #[serde_as(as = "Vec<DisplayFromStr>")]
    #[serde(default)]
    os: Vec<OS>,

    #[serde_as(as = "Vec<DisplayFromStr>")]
    systems: Vec<System>,
}

impl Default for Build {
    fn default() -> Self {
        Self {
            outputs: default_outputs(),
            artifacts: vec![SymbolicOutput::new(
                NamePattern::Specified(s!("packages")),
                SystemPattern::Any,
                NamePattern::Not(s!("formatter")),
            )],
            os: Vec::new(),
            architectures: Vec::new(),
            systems: vec![System::x86_linux(), System::x86_darwin()],
        }
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct Cache {
    #[serde(rename = "cache-name")]
    cache_name: String,

    #[serde(default = "default_publish")]
    publish: bool,

    #[serde_as(as = "Vec<DisplayFromStr>")]
    #[serde(default)]
    pin: Vec<SymbolicOutput>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct OutputConfig {
    #[serde_as(as = "DisplayFromStr")]
    #[serde(rename = "output")]
    name: SymbolicOutput,

    #[serde_as(as = "Vec<DisplayFromStr>")]
    #[serde(rename = "extra-prereqs")]
    extra_prereqs: Vec<SymbolicOutput>,
}

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    general: General,
    #[serde(rename = "cachix")]
    cache: Option<Cache>,
    #[serde(default)]
    build: Build,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default, rename = "output")]
    outputs: Vec<OutputConfig>,
}

impl Config {
    pub fn from_file(config_file: &Path) -> Result<Self> {
        let contents = fs::read_to_string(config_file)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    pub fn publish(&self) -> bool {
        let Some(cache_settings) = &self.cache else {
            return false;
        };
        cache_settings.publish
    }

    pub fn cache(&self) -> Option<&String> {
        let Some(cache_settings) = &self.cache else {
            return None;
        };
        Some(&cache_settings.cache_name)
    }

    pub fn pins(&self) -> Vec<SymbolicOutput> {
        let Some(cache_settings) = &self.cache else {
            return Vec::new();
        };
        cache_settings.pin.clone()
    }

    pub fn artifact_dir(&self) -> &String {
        &self.general.artifact_dir
    }

    pub fn env(&self) -> &HashMap<String, String> {
        &self.env
    }

    pub fn build_outputs(&self) -> &[String] {
        &self.build.outputs
    }

    pub fn systems(&self) -> Vec<System> {
        let mut systems = HashSet::new();
        for system in &self.build.systems {
            systems.insert(*system);
        }

        for arch in &self.build.architectures {
            for os in &self.build.os {
                let system = System::new(*arch, *os);
                systems.insert(system);
            }
        }

        systems.into_iter().collect()
    }

    pub fn save_artifact(&self, top_level: &String, system: System, name: &String) -> bool {
        for a in &self.build.artifacts {
            if !a.matches(top_level, system, name) {
                return false;
            }
        }
        true
    }

    pub fn output_configs<'a>(&'a self) -> HashMap<&'a SymbolicOutput, &'a Vec<SymbolicOutput>> {
        let mut map = HashMap::new();
        for output in &self.outputs {
            let name = &output.name;
            let prereqs = &output.extra_prereqs;
            map.insert(name, prereqs);
        }
        map
    }
}
