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

#[derive(Debug)]
pub struct ParseError {
    message: String,
    span: std::ops::Range<usize>,
    input: String,
}

impl ParseError {
    fn from_parse(error: &winnow::error::ParseError<&str, winnow::error::ContextError>) -> Self {
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

fn system(s: &mut &str) -> winnow::Result<System> {
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

#[derive(Debug, PartialEq, Eq, Clone)]
enum Pattern<T> {
    Any,
    Not(T),
    Specified(T),
}

type SystemPattern = Pattern<System>;
type NamePattern = Pattern<String>;

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

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct OutputPath {
    top_level: NamePattern,
    system: SystemPattern,
    name: NamePattern,
}

impl OutputPath {
    fn matches(&self, top_level: &String, system: System, name: &String) -> bool {
        self.top_level.matches(top_level) && self.system.matches(&system) && self.name.matches(name)
    }
}

fn name(s: &mut &str) -> winnow::Result<String> {
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

fn output_path(s: &mut &str) -> winnow::Result<OutputPath> {
    winnow::combinator::seq! {OutputPath {
        top_level: name_pattern,
        _: ".",
        system: system_pattern,
        _: ".",
        name: name_pattern
    }}
    .parse_next(s)
}

impl FromStr for OutputPath {
    type Err = ParseError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        output_path.parse(s).map_err(|e| ParseError::from_parse(&e))
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct Build {
    #[serde(default = "default_outputs")]
    outputs: Vec<String>,

    #[serde_as(as = "Vec<DisplayFromStr>")]
    #[serde(default)]
    artifacts: Vec<OutputPath>,

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
            artifacts: vec![OutputPath {
                top_level: Pattern::Specified(s!("packages")),
                system: SystemPattern::Any,
                name: Pattern::Not(s!("formatter")),
            }],
            os: Vec::new(),
            architectures: Vec::new(),
            systems: vec![
                System {
                    os: OS::Linux,
                    arch: Arch::X86,
                },
                System {
                    os: OS::Darwin,
                    arch: Arch::X86,
                },
            ],
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
    pin: Vec<OutputPath>,
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

    pub fn pins(&self) -> Vec<OutputPath> {
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
                let system = System {
                    arch: *arch,
                    os: *os,
                };
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
        let expected = OutputPath {
            top_level: Pattern::Specified(s!("packages")),
            system: SystemPattern::Any,
            name: Pattern::Not(s!("formatter")),
        };
        let actual = output_path.parse_next(&mut input).unwrap();
        assert_eq!(expected, actual);
        assert_eq!("", input)
    }
}
