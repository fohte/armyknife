//! Tmux session and window management.

use std::path::Path;
use std::process::Command;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum TmuxError {
    #[error("Command failed: {0}")]
    CommandFailed(String),
}

pub type Result<T> = std::result::Result<T, TmuxError>;

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
    Command::new("tmux")
        .args(["has-session", "-t", session])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Ensure a tmux session exists, creating it if necessary.
pub fn ensure_session(session: &str, cwd: &str) -> Result<()> {
    if session_exists(session) {
        return Ok(());
    }

    let output = Command::new("tmux")
        .args(["new-session", "-ds", session, "-c", cwd])
        .output()
        .map_err(|e| TmuxError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed(format!(
            "tmux new-session failed: {}",
            stderr.trim()
        )));
    }

    Ok(())
}

/// Get the current tmux session name.
pub fn current_session() -> Option<String> {
    if !in_tmux() {
        return None;
    }

    let output = Command::new("tmux")
        .args(["display-message", "-p", "#{session_name}"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
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

    let output = Command::new("tmux")
        .args(["switch-client", "-t", target_session])
        .output()
        .map_err(|e| TmuxError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed(format!(
            "tmux switch-client failed: {}",
            stderr.trim()
        )));
    }

    Ok(())
}

/// Get the current pane's working directory.
pub fn current_pane_path() -> Option<String> {
    if !in_tmux() {
        return None;
    }

    let output = Command::new("tmux")
        .args(["display-message", "-p", "#{pane_current_path}"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Get the current window ID.
pub fn current_window_id() -> Option<String> {
    if !in_tmux() {
        return None;
    }

    let output = Command::new("tmux")
        .args(["display-message", "-p", "#{window_id}"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
    Command::new("tmux")
        .args(["kill-window", "-t", window_id])
        .status()
        .ok();
}

/// Create a new window in a session.
#[allow(dead_code)]
pub fn new_window(session: &str, cwd: &str, window_name: &str) -> Result<()> {
    let output = Command::new("tmux")
        .args(["new-window", "-t", session, "-c", cwd, "-n", window_name])
        .output()
        .map_err(|e| TmuxError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed(format!(
            "tmux new-window failed: {}",
            stderr.trim()
        )));
    }

    Ok(())
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
    let mut args = Vec::new();
    for (i, cmd) in commands.iter().enumerate() {
        if i > 0 {
            args.push(";");
        }
        args.extend_from_slice(cmd);
    }

    let output = Command::new("tmux")
        .args(&args)
        .output()
        .map_err(|e| TmuxError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed(format!(
            "tmux new-window failed: {}",
            stderr.trim()
        )));
    }

    Ok(())
}
