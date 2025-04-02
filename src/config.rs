use anyhow::{bail, Result};
use s_string::s;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::{env, fs};
use toml::Value;

fn default_outputs() -> Vec<String> {
    vec![
        s!("checks"),
        s!("packages"),
        s!("devShells"),
        s!("homeConfigurations"),
        s!("darwinConfigurations"),
        s!("nixosConfigurations"),
    ]
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub env: HashMap<String, String>,
    #[serde(default = "default_outputs")]
    pub outputs: Vec<String>,
}

impl Config {
    pub fn from_file(config_file: &Path) -> Result<Self> {
        let contents = fs::read_to_string(config_file)?;
        let config: Config = toml::from_str(&contents)?;

        Ok(config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            env: HashMap::new(),
            outputs: default_outputs(),
        }
    }
}
