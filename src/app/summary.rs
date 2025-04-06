use owo_colors::{OwoColorize, Style};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

const INDENT: &str = "  ";
const STATUS_PREFIX: &str = "> ";
const SUBSTATUS_PREFIX: &str = "- ";

fn rel_to_cwd(p: &Path, cwd: &Path) -> String {
    let mut diff = pathdiff::diff_paths(p, cwd).unwrap().display().to_string();
    if !diff.starts_with('.') {
        diff = format!("./{diff}");
    }
    diff
}

fn register<T>(map: &mut HashMap<String, Vec<T>>, output_name: &str, job: T) {
    if !map.contains_key(output_name) {
        map.insert(output_name.to_string(), Vec::new());
    }
    map.get_mut(output_name).unwrap().push(job);
}

#[derive(Debug)]
pub struct Summary {
    cwd: PathBuf,
    skipped_outputs: Vec<String>,
    successes: HashMap<String, Vec<(String, Option<PathBuf>)>>,
    fails: HashMap<String, Vec<(String, String)>>,
    skips: HashMap<String, Vec<String>>,
    blocks: HashMap<String, Vec<(String, String)>>,
    nix_version: String,
    cachix_version: Option<String>,
    git_revision: String,
    width: usize,
}

impl Summary {
    pub fn new(
        cwd: PathBuf,
        nix_version: String,
        cachix_version: Option<String>,
        git_revision: String,
        width: usize,
    ) -> Self {
        Self {
            cwd,
            skipped_outputs: Vec::new(),
            successes: HashMap::new(),
            fails: HashMap::new(),
            skips: HashMap::new(),
            blocks: HashMap::new(),
            nix_version,
            git_revision,
            cachix_version,
            width,
        }
    }

    pub fn skip_output(&mut self, output: &str) {
        self.skipped_outputs.push(output.to_string());
    }

    pub fn register_success(
        &mut self,
        output_name: &str,
        job_name: String,
        artifact: Option<PathBuf>,
    ) {
        register(&mut self.successes, output_name, (job_name, artifact));
    }

    pub fn register_fail(&mut self, output_name: &str, job_name: String, log_command: String) {
        register(&mut self.fails, output_name, (job_name, log_command));
    }

    pub fn register_skip(&mut self, output_name: &str, job_name: String) {
        register(&mut self.skips, output_name, job_name);
    }

    pub fn register_blocked(&mut self, output_name: &str, job_name: String, pre_rec: String) {
        register(&mut self.blocks, output_name, (job_name, pre_rec));
    }

    fn print_line(left: &str, right: &str, style: Option<&Style>, extra_note: Option<&str>) {
        let extra_note = match extra_note {
            Some(note) => &format!(" {note}"),
            None => "",
        };

        let used_space = left.len() + right.len() + extra_note.len();
        //assert!(, "Line too big");

        let dots = if right.is_empty() {
            String::new()
        } else {
            let n = if used_space < crate::MAX_WIDTH {
                crate::MAX_WIDTH - used_space
            } else {
                10
            };
            ".".repeat(n)
        };

        match style {
            Some(style) => {
                println!(
                    "{left}{dots}{}{extra_note}",
                    right.if_supports_color(owo_colors::Stream::Stdout, |text| text.style(*style)),
                );
            }
            None => {
                println!("{left}{dots}{right}{extra_note}");
            }
        };
    }

    fn print_status_line(left: &str, right: &str, style: Option<&Style>, extra_note: Option<&str>) {
        assert_eq!(STATUS_PREFIX.len(), INDENT.len());
        let left = format!("{STATUS_PREFIX}{left}");
        Summary::print_line(&left, right, style, extra_note);
    }

    fn print_substatus_line(left: &str, right: &str, style: &Style, extra_note: Option<&str>) {
        assert_eq!(SUBSTATUS_PREFIX.len(), INDENT.len());
        let left = format!("{INDENT}{SUBSTATUS_PREFIX}{left}");
        Summary::print_line(&left, right, Some(style), extra_note);
    }

    fn print_substatus_attribute(name: &str, attribute: &str) {
        println!("{INDENT}{INDENT}{name}: {attribute}");
    }

    fn print_version(slug: &str, version: &str) {
        println!(
            "{slug}: {}",
            version.if_supports_color(owo_colors::Stream::Stdout, |text| text.bold())
        );
    }

    pub fn print(&self) {
        let yellow = Style::new().yellow().bold();
        let green = Style::new().green().bold();
        let red = Style::new().red().bold();

        let bar = "=".repeat(self.width);
        println!("{bar}");
        println!("Summary");

        // TODO: I think I'd rather mix failed/skipped/passed output and print by top_level instead

        for output in &self.skipped_outputs {
            Summary::print_status_line(output, "skipped", Some(&yellow), Some("(not found)"));
        }

        for (output, jobs) in &self.successes {
            Summary::print_status_line(output, "", None, None);
            for (job_name, artifact) in jobs {
                Summary::print_substatus_line(job_name, "success", &green, None);

                if let Some(artifact) = artifact {
                    let artifact = rel_to_cwd(artifact, &self.cwd);
                    Summary::print_substatus_attribute("artifact", &artifact);
                }
            }
        }

        for (output, jobs) in &self.skips {
            Summary::print_status_line(output, "", None, None);
            for job in jobs {
                Summary::print_substatus_line(job, "skipped", &yellow, Some("(dry run)"));
            }
        }

        for (output, jobs) in &self.blocks {
            Summary::print_status_line(output, "", None, None);
            for (job, pre_rec) in jobs {
                Summary::print_substatus_line(
                    job,
                    "skipped",
                    &yellow,
                    Some(&format!("(pre-rec '{pre_rec}' failed)")),
                );
            }
        }

        for (output, jobs) in &self.fails {
            println!("> {output}");
            for (job, log_command) in jobs {
                Summary::print_substatus_line(job, "failed", &red, None);
                Summary::print_substatus_attribute("log command", log_command);
            }
        }

        Summary::print_version("Git revision", &self.git_revision);
        Summary::print_version("Nix version:", &self.nix_version);
        if let Some(cachix_version) = &self.cachix_version {
            Summary::print_version("Cachix version", cachix_version);
        };
    }
}
