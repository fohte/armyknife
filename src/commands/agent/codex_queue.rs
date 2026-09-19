//! Queues a message onto a Codex session with `codex queue`, the Codex
//! counterpart to `claude_messaging`.
//!
//! Unlike Claude Code's socket write, this is not a direct delivery. `codex
//! queue` only appends to the thread's queue in a SQLite DB under
//! `$CODEX_HOME` (so a mismatched `$CODEX_HOME` queues into a DB nobody
//! reads); the interactive `codex` process that owns the thread polls that DB
//! every 10 seconds and starts a turn once the thread is idle. A successful
//! return therefore means "queued", not "delivered".

use std::io::ErrorKind;
use std::process::Output;

use anyhow::{Result, anyhow, bail};

use crate::shared::command;

/// Queues `content` as a user message on the Codex thread `session_id`.
///
/// Every failure is surfaced, including the ones `codex queue` itself
/// reports: the thread is archived, or a local app-server daemon is running
/// (it cannot queue through the embedded app server in that case).
pub fn queue_message(session_id: &str, content: &str) -> Result<()> {
    let output = command::new("codex")
        .args(queue_args(session_id, content))
        .output()
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => anyhow!("Could not find 'codex' command in PATH"),
            _ => anyhow!(e).context("failed to run `codex queue`"),
        })?;
    ensure_success(&output)
}

/// `--message=` is joined with `=` so a message starting with `-` is not
/// parsed as a flag.
fn queue_args(session_id: &str, content: &str) -> Vec<String> {
    vec![
        "queue".to_string(),
        "--thread".to_string(),
        session_id.to_string(),
        format!("--message={content}"),
    ]
}

fn ensure_success(output: &Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "`codex queue` failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::plain("abc", "hello", &["queue", "--thread", "abc", "--message=hello"])]
    #[case::leading_hyphen("abc", "-rf", &["queue", "--thread", "abc", "--message=-rf"])]
    fn queue_args_cases(
        #[case] session_id: &str,
        #[case] content: &str,
        #[case] expected: &[&str],
    ) {
        assert_eq!(queue_args(session_id, content), expected);
    }

    #[rstest]
    #[case::success(0, "", Ok(()))]
    #[case::failure(
        1 << 8,
        "Error: session abc is archived.\n",
        Err("`codex queue` failed (exit status: 1): Error: session abc is archived.")
    )]
    fn ensure_success_cases(
        #[case] raw_status: i32,
        #[case] stderr: &str,
        #[case] expected: std::result::Result<(), &str>,
    ) {
        let output = Output {
            status: ExitStatus::from_raw(raw_status),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        };
        assert_eq!(
            ensure_success(&output).map_err(|e| e.to_string()),
            expected.map_err(str::to_string),
        );
    }
}
