//! Helpers for invoking the `git` CLI.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};

use super::error::{GitError, Result};

/// Build a `git` command with `-C <dir>` so it operates against the given path.
pub fn git_at(dir: &Path) -> Command {
    let mut cmd = crate::shared::command::new("git");
    cmd.arg("-C").arg(dir);
    cmd
}

/// Run a `git` subcommand against `dir`, returning trimmed stdout on success.
///
/// `args` are passed verbatim after `-C <dir>`. On non-zero exit, returns
/// [`GitError::CommandFailed`] with the captured stderr.
pub fn run_git<I, S>(dir: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = git_at(dir);
    for arg in args {
        cmd.arg(arg);
    }
    let output = cmd.output().map_err(GitError::SpawnFailed)?;
    check_output(output)
}

/// Run a global `git` command (no `-C`).
pub fn run_git_global<I, S>(args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = crate::shared::command::new("git");
    for arg in args {
        cmd.arg(arg);
    }
    let output = cmd.output().map_err(GitError::SpawnFailed)?;
    check_output(output)
}

fn check_output(output: Output) -> Result<String> {
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        Ok(stdout.trim_end_matches('\n').to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(GitError::CommandFailed(stderr).into())
    }
}

/// Run a `git` command and return Ok regardless of exit status.
///
/// Useful for queries like `rev-parse` where failure indicates "not present"
/// rather than a real error.
pub fn run_git_optional<I, S>(dir: &Path, args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_git(dir, args).ok()
}
