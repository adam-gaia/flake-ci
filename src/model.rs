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

const LINUX: &str = "linux";
const DARWIN: &str = "darwin";
const WINDOWS: &str = "windows";
const ARM: &str = "aarch64";
const X86: &str = "x86_64";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Derivation {
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
    pub fn output(&self) -> &String {
        &self.output
    }
    pub fn system(&self) -> &System {
        &self.system
    }
    pub fn name(&self) -> &String {
        &self.name
    }
}

fn derivation(s: &mut &str) -> winnow::Result<Derivation> {
    winnow::combinator::seq! {Derivation {
        output: name,
        _: ".",
        system:  system,
        _: ".",
        name:  name,
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

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Status {
    Skipped,
    Success,
    Fail,
}

impl Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Skipped => "skipped",
            Self::Success => "success",
            Self::Fail => "failed",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug)]
pub struct ParseError {
    message: String,
    span: std::ops::Range<usize>,
    input: String,
}

impl ParseError {
    pub fn from_parse(
        error: &winnow::error::ParseError<&str, winnow::error::ContextError>,
    ) -> Self {
        let message = error.inner().to_string();
        let input = (*error.input()).to_owned();
        let span = error.char_span();
        Self {
            message,
            span,
            input,
        }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = annotate_snippets::Level::Error
            .title(&self.message)
            .snippet(
                annotate_snippets::Snippet::source(&self.input)
                    .fold(true)
                    .annotation(annotate_snippets::Level::Error.span(self.span.clone())),
            );
        let renderer = annotate_snippets::Renderer::plain();
        let rendered_message = renderer.render(message);
        rendered_message.fmt(f)
    }
}

impl std::error::Error for ParseError {}
#[derive(Debug, Serialize, Deserialize, Eq, PartialEq, Clone, Copy, Hash)]
pub enum OS {
    Linux,
    Darwin,
    Windows,
}

impl Display for OS {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Linux => write!(f, "{LINUX}"),
            Self::Darwin => write!(f, "{DARWIN}"),
            Self::Windows => write!(f, "{WINDOWS}"),
        }
    }
}

fn os(s: &mut &str) -> winnow::Result<OS> {
    winnow::combinator::alt((LINUX.map(|_| OS::Linux), DARWIN.map(|_| OS::Darwin))).parse_next(s)
}

impl FromStr for OS {
    type Err = ParseError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        os.parse(s).map_err(|e| ParseError::from_parse(&e))
    }
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq, Clone, Copy, Hash)]
pub enum Arch {
    X86,
    Arm,
}

impl Display for Arch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::X86 => write!(f, "{X86}"),
            Self::Arm => write!(f, "{ARM}"),
        }
    }
}

fn arch(s: &mut &str) -> winnow::Result<Arch> {
    winnow::combinator::alt((X86.map(|_| Arch::X86), ARM.map(|_| Arch::Arm))).parse_next(s)
}

impl FromStr for Arch {
    type Err = ParseError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        arch.parse(s).map_err(|e| ParseError::from_parse(&e))
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash)]
pub struct System {
    os: OS,
    arch: Arch,
}

impl System {
    pub fn new(arch: Arch, os: OS) -> Self {
        Self { arch, os }
    }
    pub fn x86_linux() -> Self {
        Self {
            os: OS::Linux,
            arch: Arch::X86,
        }
    }

    pub fn x86_darwin() -> Self {
        Self {
            os: OS::Darwin,
            arch: Arch::X86,
        }
    }

    pub fn arm_linux() -> Self {
        Self {
            os: OS::Linux,
            arch: Arch::Arm,
        }
    }

    pub fn arm_darwin() -> Self {
        Self {
            os: OS::Darwin,
            arch: Arch::Arm,
        }
    }

    pub fn x86_windows() -> Self {
        Self {
            os: OS::Windows,
            arch: Arch::X86,
        }
    }

    pub fn arm_windows() -> Self {
        Self {
            os: OS::Windows,
            arch: Arch::Arm,
        }
    }
}

impl Display for System {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.arch, self.os)
    }
}

pub fn system(s: &mut &str) -> winnow::Result<System> {
    winnow::combinator::seq! {System {
        arch: arch,
        _: "-",
        os: os
    }}
    .parse_next(s)
}

impl FromStr for System {
    type Err = ParseError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        system.parse(s).map_err(|e| ParseError::from_parse(&e))
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub enum Pattern<T> {
    Any,
    Not(T),
    Specified(T),
}

pub type SystemPattern = Pattern<System>;
pub type NamePattern = Pattern<String>;

impl<T> Pattern<T>
where
    T: Eq + PartialEq,
{
    pub fn matches(&self, other: &T) -> bool {
        match self {
            Self::Any => true,
            Self::Not(pattern) => other != pattern,
            Self::Specified(pattern) => other == pattern,
        }
    }
}

impl<T> Display for Pattern<T>
where
    T: Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Any => "*".to_string(),
            Self::Not(pattern) => format!("!{pattern}"),
            Self::Specified(pattern) => pattern.to_string(),
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct SymbolicOutput {
    top_level: NamePattern,
    system: SystemPattern,
    name: NamePattern,
}

impl SymbolicOutput {
    pub fn new(output: NamePattern, system: SystemPattern, name: NamePattern) -> Self {
        Self {
            top_level: output,
            system,
            name,
        }
    }

    pub fn matches(&self, top_level: &String, system: System, name: &String) -> bool {
        self.top_level.matches(top_level) && self.system.matches(&system) && self.name.matches(name)
    }

    pub fn top_level(&self) -> &NamePattern {
        &self.top_level
    }

    pub fn system(&self) -> &SystemPattern {
        &self.system
    }

    pub fn name(&self) -> &NamePattern {
        &self.name
    }

    pub fn matrix(
        &self,
        all_top_levels: &[&String],
        all_systems: &[System],
        all_derivations: &[&Derivation],
    ) -> Result<Vec<Derivation>> {
        let outputs: Vec<String> = match self.top_level() {
            NamePattern::Any => all_top_levels.iter().map(|x| (*x).clone()).collect(),
            NamePattern::Not(name) => all_top_levels
                .iter()
                .map(|x| (*x).clone())
                .filter(|x| x != name)
                .collect(),
            NamePattern::Specified(name) => {
                vec![name.clone()]
            }
        };

        let systems: Vec<System> = match self.system() {
            SystemPattern::Any => all_systems.to_vec(),
            SystemPattern::Not(system) => all_systems
                .iter()
                .map(|x| x.to_owned())
                .filter(|x| x != system)
                .collect(),
            SystemPattern::Specified(system) => {
                vec![system.clone()]
            }
        };

        let mut drvs = Vec::new();
        for output in &outputs {
            for system in &systems {
                let relevant: Vec<&Derivation> = all_derivations
                    .iter()
                    .map(|x| *x)
                    .filter(|drv| drv.output() == output && drv.system() == system)
                    .collect();
                if relevant.is_empty() {
                    bail!("No outputs match {self}")
                };

                let mut foos: Vec<Derivation> = match self.name() {
                    NamePattern::Any => relevant.iter().map(|x| (*x).clone()).collect(),

                    NamePattern::Not(name) => relevant
                        .iter()
                        .map(|x| (*x).clone())
                        .filter(|x| x.name() != name)
                        .collect(),
                    NamePattern::Specified(name) => relevant
                        .iter()
                        .map(|x| (*x).clone())
                        .filter(|x| x.name() == name)
                        .collect(),
                };

                drvs.append(&mut foos);
            }
        }

        Ok(drvs)
    }
}

impl Display for SymbolicOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.top_level, self.system, self.name)
    }
}

pub fn name(s: &mut &str) -> winnow::Result<String> {
    winnow::token::take_while(1.., |c: char| c.is_alphanum() || c == '_' || c == '-') // TODO: are dashes and underscores valid?
        .map(|s: &str| String::from(s))
        .parse_next(s)
}

fn star(s: &mut &str) -> winnow::Result<()> {
    let _ = "*".parse_next(s)?;
    Ok(())
}

fn not(s: &mut &str) -> winnow::Result<String> {
    let _ = "!".parse_next(s)?;
    name.parse_next(s)
}

fn not_system(s: &mut &str) -> winnow::Result<System> {
    let _ = "!".parse_next(s)?;
    system.parse_next(s)
}

fn name_pattern(s: &mut &str) -> winnow::Result<NamePattern> {
    winnow::combinator::alt((
        star.map(|()| NamePattern::Any),
        not.map(NamePattern::Not),
        name.map(NamePattern::Specified),
    ))
    .parse_next(s)
}

fn system_pattern(s: &mut &str) -> winnow::Result<SystemPattern> {
    winnow::combinator::alt((
        star.map(|()| SystemPattern::Any),
        not_system.map(SystemPattern::Not),
        system.map(SystemPattern::Specified),
    ))
    .parse_next(s)
}

fn output_path(s: &mut &str) -> winnow::Result<SymbolicOutput> {
    winnow::combinator::seq! {SymbolicOutput {
        top_level: name_pattern,
        _: ".",
        system: system_pattern,
        _: ".",
        name: name_pattern
    }}
    .parse_next(s)
}

impl FromStr for SymbolicOutput {
    type Err = ParseError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        output_path.parse(s).map_err(|e| ParseError::from_parse(&e))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_parse_any_pattern() {
        let mut input = "*";
        let expected = Pattern::Any;
        let actual = name_pattern.parse_next(&mut input).unwrap();
        assert_eq!(expected, actual);
        assert_eq!("", input)
    }

    #[test]
    fn test_parse_not_pattern() {
        let mut input = "!formatter";
        let expected = Pattern::Not(s!("formatter"));
        let actual = name_pattern.parse_next(&mut input).unwrap();
        assert_eq!(expected, actual);
        assert_eq!("", input)
    }

    #[test]
    fn test_parse_pattern() {
        let mut input = "packages";
        let expected = Pattern::Specified(s!("packages"));
        let actual = name_pattern.parse_next(&mut input).unwrap();
        assert_eq!(expected, actual);
        assert_eq!("", input)
    }

    #[test]
    fn test_parse_output_path() {
        let mut input = "packages.*.!formatter";
        let expected = SymbolicOutput {
            top_level: Pattern::Specified(s!("packages")),
            system: SystemPattern::Any,
            name: Pattern::Not(s!("formatter")),
        };
        let actual = output_path.parse_next(&mut input).unwrap();
        assert_eq!(expected, actual);
        assert_eq!("", input)
    }
}
