//! Tmux session and window management.

use std::fmt;
use std::path::Path;
use std::process::Command;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum TmuxError {
    #[error("{}", .0)]
    CommandFailed(CommandFailedError),

    #[error("Not running inside tmux")]
    NotInTmux,
}

#[derive(Debug)]
pub struct CommandFailedError {
    pub command: String,
    pub args: Vec<String>,
    pub message: String,
    pub stderr: Option<String>,
}

impl fmt::Display for CommandFailedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} failed: {}",
            self.command,
            self.args.join(" "),
            self.message
        )?;
        if let Some(stderr) = &self.stderr
            && !stderr.is_empty()
        {
            write!(f, "\n-- stderr --\n{stderr}")?;
        }
        Ok(())
    }
}

impl TmuxError {
    fn command_failed(args: &[&str], message: impl Into<String>, stderr: Option<String>) -> Self {
        Self::CommandFailed(CommandFailedError {
            command: "tmux".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            message: message.into(),
            stderr,
        })
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

/// Run a tmux command that requires being inside a tmux session.
/// Returns an error if not running inside tmux.
fn run_tmux_in_session(args: &[&str]) -> Result<()> {
    if !in_tmux() {
        return Err(TmuxError::NotInTmux);
    }
    run_tmux(args)
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
/// Returns `org/repo` format from the last two path components.
/// Strips `/.worktrees/<name>` suffix if present.
/// Replaces `.` with `_` to comply with tmux session name conventions.
pub fn get_session_name(repo_root: &str) -> String {
    let path = Path::new(repo_root);

    // Strip /.worktrees/<name> suffix if present
    let path = strip_worktree_suffix(path);

    // Extract org/repo from path (last two components)
    let repo = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");
    let org = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    format!("{org}/{repo}").replace('.', "_")
}

/// Strip `/.worktrees/<name>` suffix from a path.
fn strip_worktree_suffix(path: &Path) -> &Path {
    use std::ffi::OsStr;

    // Check if path ends with /.worktrees/<something>
    if let Some(parent) = path.parent()
        && parent.file_name() == Some(OsStr::new(".worktrees"))
        && let Some(repo_root) = parent.parent()
    {
        return repo_root;
    }
    path
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

/// Switch to a different tmux session.
/// Returns an error if not running inside tmux.
pub fn switch_to_session(target_session: &str) -> Result<()> {
    if let Some(current) = current_session()
        && current == target_session
    {
        return Ok(());
    }

    run_tmux_in_session(&["switch-client", "-t", target_session])
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
pub fn kill_window(window_id: &str) -> Result<()> {
    run_tmux(&["kill-window", "-t", window_id])
}

/// Select a tmux window by target (e.g., "session:0" or "session:window_name").
/// Returns an error if not running inside tmux.
pub fn select_window(target: &str) -> Result<()> {
    run_tmux_in_session(&["select-window", "-t", target])
}

/// Select a tmux pane by ID (e.g., "%0").
/// Returns an error if not running inside tmux.
pub fn select_pane(pane_id: &str) -> Result<()> {
    run_tmux_in_session(&["select-pane", "-t", pane_id])
}

/// Information about a tmux pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    pub session_name: String,
    pub window_name: String,
    pub window_index: u32,
    pub pane_id: String,
}

/// Gets tmux pane information for a given TTY device.
/// Returns None if not running in tmux or if the TTY is not found.
pub fn get_pane_info_by_tty(tty: &str) -> Option<PaneInfo> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{pane_tty}\t#{session_name}\t#{window_name}\t#{window_index}\t#{pane_id}",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| parse_pane_line(line, tty))
}

/// Parses a single line from tmux list-panes output.
/// Format: "#{pane_tty}\t#{session_name}\t#{window_name}\t#{window_index}\t#{pane_id}"
fn parse_pane_line(line: &str, target_tty: &str) -> Option<PaneInfo> {
    let mut parts = line.split('\t');

    let pane_tty = parts.next()?;
    if pane_tty != target_tty {
        return None;
    }

    let session_name = parts.next()?.to_string();
    let window_name = parts.next()?.to_string();
    let window_index = parts.next()?.parse::<u32>().ok()?;
    let pane_id = parts.next()?.to_string();

    Some(PaneInfo {
        session_name,
        window_name,
        window_index,
        pane_id,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("/Users/fohte/ghq/github.com/fohte/armyknife", "fohte/armyknife")]
    #[case("/Users/fohte/ghq/github.com/fohte/dotfiles", "fohte/dotfiles")]
    #[case("/home/user/projects/org/my-repo", "org/my-repo")]
    fn test_get_session_name_normal_path(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(get_session_name(input), expected);
    }

    #[rstest]
    #[case(
        "/Users/fohte/ghq/github.com/fohte/armyknife/.worktrees/feature-branch",
        "fohte/armyknife"
    )]
    #[case(
        "/Users/fohte/ghq/github.com/fohte/dotfiles/.worktrees/fix-bug",
        "fohte/dotfiles"
    )]
    fn test_get_session_name_worktree_path(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(get_session_name(input), expected);
    }

    #[rstest]
    #[case("/Users/fohte/ghq/github.com/fohte/my.dotfiles", "fohte/my_dotfiles")]
    #[case("/Users/fohte/ghq/github.com/some.org/some.repo", "some_org/some_repo")]
    fn test_get_session_name_replaces_dots(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(get_session_name(input), expected);
    }

    #[rstest]
    #[case(
        "/Users/fohte/ghq/github.com/fohte/armyknife/.worktrees/feature-branch",
        "/Users/fohte/ghq/github.com/fohte/armyknife"
    )]
    #[case(
        "/Users/fohte/ghq/github.com/fohte/dotfiles/.worktrees/fix-bug",
        "/Users/fohte/ghq/github.com/fohte/dotfiles"
    )]
    fn test_strip_worktree_suffix_with_worktree(#[case] input: &str, #[case] expected: &str) {
        let result = strip_worktree_suffix(Path::new(input));
        assert_eq!(result, Path::new(expected));
    }

    #[rstest]
    #[case("/Users/fohte/ghq/github.com/fohte/armyknife")]
    #[case("/Users/fohte/projects/myrepo")]
    fn test_strip_worktree_suffix_without_worktree(#[case] input: &str) {
        let result = strip_worktree_suffix(Path::new(input));
        assert_eq!(result, Path::new(input));
    }

    #[rstest]
    #[case::standard_line(
        "/dev/ttys001\tmain\teditor\t0\t%0",
        "/dev/ttys001",
        Some(("main", "editor", 0, "%0"))
    )]
    #[case::different_tty("/dev/ttys002\twork\tterminal\t1\t%5", "/dev/ttys001", None)]
    #[case::high_window_index(
        "/dev/pts/0\tsession\twindow\t99\t%123",
        "/dev/pts/0",
        Some(("session", "window", 99, "%123"))
    )]
    #[case::session_with_spaces(
        "/dev/ttys001\tmy session\tmy window\t0\t%0",
        "/dev/ttys001",
        Some(("my session", "my window", 0, "%0"))
    )]
    #[case::insufficient_parts("/dev/ttys001\tmain", "/dev/ttys001", None)]
    #[case::empty_line("", "/dev/ttys001", None)]
    fn test_parse_pane_line(
        #[case] line: &str,
        #[case] target_tty: &str,
        #[case] expected: Option<(&str, &str, u32, &str)>,
    ) {
        let result = parse_pane_line(line, target_tty);

        match expected {
            Some((session, window, index, pane)) => {
                let info = result.expect("expected Some(PaneInfo)");
                assert_eq!(info.session_name, session);
                assert_eq!(info.window_name, window);
                assert_eq!(info.window_index, index);
                assert_eq!(info.pane_id, pane);
            }
            None => assert!(result.is_none()),
        }
    }
}
