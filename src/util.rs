use crate::config::Config;
use crate::graph::Graph;
use crate::model::Derivation;
use crate::model::{NamePattern, SymbolicOutput, System, SystemPattern};
use crate::nix::run;
use crate::Settings;
use anyhow::{bail, Result};
use log::{debug, warn};
use owo_colors::OwoColorize;
use std::cmp::Eq;
use std::fmt::{Debug, Display};
use std::ops::IndexMut;
use std::path::{Path, PathBuf};
use std::{collections::HashMap, hash::Hash};

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
        _ => bail!("Unknown check type '{input}'"),
    };

    Ok(res)
}
pub fn get_type_of_check(derivation: &Derivation) -> Result<&'static str> {
    let name = derivation.name();
    let Some((prefix, _)) = name.split_once('-') else {
        bail!("TODO: better error message");
    };

    find_check_type(&prefix)
}

pub fn parse_check_name(check: &Derivation) -> Result<(&'static str, &str)> {
    assert!(
        check.output() == "checks",
        "Passed a non-check to parse_check_type()"
    );

    let Some((prefix, name)) = check.name().split_once('-') else {
        bail!("TODO: better err message");
    };

    let ttype = find_check_type(prefix)?;
    Ok((ttype, name))
}
