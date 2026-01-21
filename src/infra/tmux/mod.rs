//! Tmux session and window management.

use std::path::Path;
use std::process::Command;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum TmuxError {
    #[error("tmux command '{command}' failed: {message}")]
    CommandFailed {
        command: String,
        args: Vec<String>,
        message: String,
        stderr: Option<String>,
    },
}

impl TmuxError {
    fn command_failed(args: &[&str], message: impl Into<String>, stderr: Option<String>) -> Self {
        Self::CommandFailed {
            command: "tmux".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            message: message.into(),
            stderr,
        }
    }
}

pub type Result<T> = std::result::Result<T, TmuxError>;

// ============================================================================
// Internal helpers for command execution
// ============================================================================

/// Run a tmux command and return stdout on success.
fn run_tmux_output(args: &[&str]) -> Result<String> {
    let output = Command::new("tmux")
        .args(args)
        .output()
        .map_err(|e| TmuxError::command_failed(args, e.to_string(), None))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(TmuxError::command_failed(
            args,
            "command exited with non-zero status",
            Some(stderr),
        ))
    }
}

/// Run a tmux command, returning Ok(()) on success.
fn run_tmux(args: &[&str]) -> Result<()> {
    run_tmux_output(args).map(|_| ())
}

/// Query a tmux value using display-message with a format string.
/// Returns None if not in tmux or if the command fails.
fn query_tmux_value(format_string: &str) -> Option<String> {
    if !in_tmux() {
        return None;
    }

    run_tmux_output(&["display-message", "-p", format_string]).ok()
}

// ============================================================================
// Public API
// ============================================================================

/// Check if running inside a tmux session.
pub fn in_tmux() -> bool {
    std::env::var("TMUX").is_ok()
}

/// Get the tmux session name for a repository path.
///
/// First tries `tmux-name session <path>`, falls back to directory basename.
pub fn get_session_name(repo_root: &str) -> String {
    // Try tmux-name command first
    if let Some(output) = Command::new("tmux-name")
        .args(["session", repo_root])
        .output()
        .ok()
        .filter(|o| o.status.success())
    {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }

    // Fallback: use the directory name
    Path::new(repo_root)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default")
        .to_string()
}

/// Check if a session exists.
pub fn session_exists(session: &str) -> bool {
    run_tmux(&["has-session", "-t", session]).is_ok()
}

/// Ensure a tmux session exists, creating it if necessary.
pub fn ensure_session(session: &str, cwd: &str) -> Result<()> {
    if session_exists(session) {
        return Ok(());
    }

    run_tmux(&["new-session", "-ds", session, "-c", cwd])
}

/// Get the current tmux session name.
pub fn current_session() -> Option<String> {
    query_tmux_value("#{session_name}")
}

/// Switch to a different tmux session (only if inside tmux).
pub fn switch_to_session(target_session: &str) -> Result<()> {
    if !in_tmux() {
        return Ok(());
    }

    if let Some(current) = current_session()
        && current == target_session
    {
        return Ok(());
    }

    run_tmux(&["switch-client", "-t", target_session])
}

/// Get the current pane's working directory.
pub fn current_pane_path() -> Option<String> {
    query_tmux_value("#{pane_current_path}")
}

/// Get the current window ID.
pub fn current_window_id() -> Option<String> {
    query_tmux_value("#{window_id}")
}

/// Get the current window ID if the pane is inside the given path.
pub fn get_window_id_if_in_path(path: &str) -> Option<String> {
    let pane_path = current_pane_path()?;

    // Use Path::starts_with for proper path comparison
    // (avoids /tmp/foo matching /tmp/foo2)
    if std::path::Path::new(&pane_path).starts_with(path) {
        current_window_id()
    } else {
        None
    }
}

/// Kill a tmux window by its ID.
pub fn kill_window(window_id: &str) {
    let _ = run_tmux(&["kill-window", "-t", window_id]);
}

/// Create a new window in a session.
#[allow(dead_code)]
pub fn new_window(session: &str, cwd: &str, window_name: &str) -> Result<()> {
    run_tmux(&["new-window", "-t", session, "-c", cwd, "-n", window_name])
}

/// Create a new window with a horizontal split and run commands in each pane.
///
/// - Left pane (pane 1): runs `left_cmd`
/// - Right pane (pane 2): runs `right_cmd`
/// - Focus ends on left pane
pub fn create_split_window(
    session: &str,
    cwd: &str,
    window_name: &str,
    left_cmd: &str,
    right_cmd: &str,
) -> Result<()> {
    // Build tmux command chain. Each sub-array is one tmux command.
    let commands: &[&[&str]] = &[
        &["new-window", "-t", session, "-c", cwd, "-n", window_name],
        &["split-window", "-h", "-c", cwd],
        &["select-pane", "-t", "1"],
        &["send-keys", left_cmd, "C-m"],
        &["select-pane", "-t", "2"],
        &["send-keys", right_cmd, "C-m"],
        &["select-pane", "-t", "1"],
    ];

    // Interleave commands with ";" separator for tmux chaining
    let mut args: Vec<&str> = Vec::new();
    for (i, cmd) in commands.iter().enumerate() {
        if i > 0 {
            args.push(";");
        }
        args.extend_from_slice(cmd);
    }

    run_tmux(&args)
}
