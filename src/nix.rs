use anyhow::bail;
use anyhow::Result;
use log::debug;
use std::collections::HashMap;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Command;

use crate::app::Status;

pub fn run(exec: &Path, args: &[&str]) -> Result<String> {
    debug!("Running command: {} {:?}", exec.display(), args);
    let output = Command::new(exec).args(args).output()?;
    let status = output.status;

    if !status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if !stderr.is_empty() {
            debug!("[stderr]{}", stderr);
        }

        let code = match status.code() {
            Some(code) => code,
            None => status
                .signal()
                .expect("Process was killed by a signal, but we couldn't get the signal type"),
        };
        bail!(
            "'{}' exited with non-zero exit code {}",
            exec.display(),
            code
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(stdout)
}

pub fn run_stream(
    exec: &Path,
    args: &[&str],
    env: Option<&HashMap<String, String>>,
    dry_run: bool,
) -> Result<Status> {
    debug!("Running command: {} {args:?}", exec.display());
    let mut cmd = &mut Command::new(exec);
    cmd = cmd.args(args);
    if let Some(env) = env {
        cmd = cmd.envs(env);
    };
    let status = if dry_run {
        println!("[DRYRUN] Would run '{cmd:?}'");
        Status::Skipped
    } else if cmd.status()?.success() {
        Status::Success
    } else {
        Status::Fail
    };
    Ok(status)
}
