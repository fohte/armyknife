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

/// Get all window IDs that have panes with working directories inside the given path.
///
/// This searches all tmux sessions and returns unique window IDs where any pane
/// has its current working directory inside `path`.
pub fn get_window_ids_in_path(path: &str) -> Vec<String> {
    let output = match run_tmux_output(&[
        "list-panes",
        "-a",
        "-F",
        "#{pane_current_path}\t#{window_id}",
    ]) {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };

    let target_path = std::path::Path::new(path);
    let mut window_ids: Vec<String> = output
        .lines()
        .filter_map(|line| {
            // Use rsplit_once to handle paths containing tabs correctly,
            // since window_id is guaranteed not to contain tabs
            let (pane_path, window_id) = line.rsplit_once('\t')?;

            if std::path::Path::new(pane_path).starts_with(target_path) {
                Some(window_id.to_string())
            } else {
                None
            }
        })
        .collect();

    // Remove duplicates (multiple panes can be in the same window)
    window_ids.sort();
    window_ids.dedup();
    window_ids
}

/// Select a tmux pane by ID (e.g., "%0").
/// Returns an error if not running inside tmux.
pub fn select_pane(pane_id: &str) -> Result<()> {
    run_tmux_in_session(&["select-pane", "-t", pane_id])
}

/// Set a user option on a specific tmux pane.
/// User options are prefixed with '@' (e.g., "@armyknife-session-id").
/// This does not require being inside tmux, as it targets a specific pane ID.
pub fn set_pane_option(pane_id: &str, option: &str, value: &str) -> Result<()> {
    run_tmux(&["set-option", "-p", "-t", pane_id, option, value])
}

/// Unset a user option on a specific tmux pane.
/// User options are prefixed with '@' (e.g., "@armyknife-session-id").
/// This does not require being inside tmux, as it targets a specific pane ID.
pub fn unset_pane_option(pane_id: &str, option: &str) -> Result<()> {
    run_tmux(&["set-option", "-p", "-u", "-t", pane_id, option])
}

/// Get a user option value from the current tmux pane.
/// Returns None if not in tmux, the option is not set, or the command fails.
pub fn get_current_pane_option(option: &str) -> Option<String> {
    if !in_tmux() {
        return None;
    }

    // Use show-options to get pane-specific option value
    // Format: "option_name value"
    let output = run_tmux_output(&["show-options", "-p", "-v", option]).ok()?;
    if output.is_empty() {
        None
    } else {
        Some(output)
    }
}

/// Get the current pane's ID (e.g., "%0").
/// Returns None if not in tmux or if the command fails.
pub fn current_pane_id() -> Option<String> {
    query_tmux_value("#{pane_id}")
}

/// Check if the tmux server is available (running and responsive).
pub fn is_server_available() -> bool {
    run_tmux_output(&["list-sessions"]).is_ok()
}

/// Checks if a tmux pane with the given pane_id exists.
/// Returns false if pane doesn't exist OR if tmux server is not available.
/// Caller should check is_server_available() first if distinction matters.
pub fn is_pane_alive(pane_id: &str) -> bool {
    // Use list-panes to check if pane exists (returns error if pane not found)
    run_tmux_output(&["list-panes", "-t", pane_id]).is_ok()
}

/// Information about a tmux pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    pub session_name: String,
    pub window_name: String,
    pub window_index: u32,
    pub pane_id: String,
}

/// Information about a tmux pane with user option values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfoWithOption {
    pub session_name: String,
    pub window_index: u32,
    pub pane_index: u32,
    pub pane_id: String,
    pub option_value: Option<String>,
}

/// Lists all tmux panes with a specific user option value.
/// Returns panes where the option is set (non-empty value).
/// The option name should include the '@' prefix (e.g., "@armyknife-session-id").
pub fn list_all_panes_with_option(option: &str) -> Vec<PaneInfoWithOption> {
    // Build format string to get pane info and option value
    // #{option} syntax retrieves the option value for each pane
    let format = format!(
        "#{{session_name}}\t#{{window_index}}\t#{{pane_index}}\t#{{pane_id}}\t#{{{}}}",
        option
    );

    let output = match run_tmux_output(&["list-panes", "-a", "-F", &format]) {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };

    output
        .lines()
        .filter_map(parse_pane_with_option_line)
        .collect()
}

/// Parses a single line from tmux list-panes output with user option.
/// Format: "#{session_name}\t#{window_index}\t#{pane_index}\t#{pane_id}\t#{option}"
/// Returns None if the line is malformed or the option is empty.
fn parse_pane_with_option_line(line: &str) -> Option<PaneInfoWithOption> {
    let mut parts = line.split('\t');

    let session_name = parts.next()?.to_string();
    let window_index = parts.next()?.parse::<u32>().ok()?;
    let pane_index = parts.next()?.parse::<u32>().ok()?;
    let pane_id = parts.next()?.to_string();
    let option_value = parts.next().map(|s| s.to_string());

    // Only include panes where the option is set
    if option_value.as_ref().is_some_and(|v| !v.is_empty()) {
        Some(PaneInfoWithOption {
            session_name,
            window_index,
            pane_index,
            pane_id,
            option_value,
        })
    } else {
        None
    }
}

/// Finds a pane by session:window_index.pane_index and returns its pane_id.
/// Returns None if the pane is not found.
pub fn find_pane_id_by_position(
    session_name: &str,
    window_index: u32,
    pane_index: u32,
) -> Option<String> {
    // Target format: session_name:window_index.pane_index
    let target = format!("{}:{}.{}", session_name, window_index, pane_index);

    // Use list-panes with target to get pane_id
    let output = run_tmux_output(&["list-panes", "-t", &target, "-F", "#{pane_id}"]).ok()?;

    if output.is_empty() {
        None
    } else {
        Some(output)
    }
}

/// Gets tmux pane information for a given process ID.
/// Searches for a pane whose pane_pid matches the given PID or any of its ancestor PIDs.
/// Returns None if not running in tmux or if no matching pane is found.
pub fn get_pane_info_by_pid(pid: u32) -> Option<PaneInfo> {
    let output = run_tmux_output(&[
        "list-panes",
        "-a",
        "-F",
        "#{pane_pid}\t#{session_name}\t#{window_name}\t#{window_index}\t#{pane_id}",
    ])
    .ok()?;

    // Collect all pane PIDs for ancestor matching
    let pane_pids: std::collections::HashSet<u32> = output
        .lines()
        .filter_map(|line| line.split('\t').next()?.parse::<u32>().ok())
        .collect();

    // Traverse ancestor processes to find one that matches a pane PID
    let mut current_pid = pid;
    const MAX_DEPTH: usize = 20;

    for _ in 0..MAX_DEPTH {
        if pane_pids.contains(&current_pid) {
            // Found a matching pane, parse the full info
            return output
                .lines()
                .find_map(|line| parse_pane_line_by_pid(line, current_pid));
        }

        // Get parent PID
        match get_parent_pid(current_pid) {
            Some(ppid) if ppid != current_pid && ppid != 0 => current_pid = ppid,
            _ => break,
        }
    }

    None
}

/// Gets the parent process ID for a given process.
fn get_parent_pid(pid: u32) -> Option<u32> {
    let output = Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .ok()
}

/// Parses a single line from tmux list-panes output for PID matching.
/// Format: "#{pane_pid}\t#{session_name}\t#{window_name}\t#{window_index}\t#{pane_id}"
fn parse_pane_line_by_pid(line: &str, target_pid: u32) -> Option<PaneInfo> {
    let mut parts = line.split('\t');

    let pane_pid: u32 = parts.next()?.parse().ok()?;
    if pane_pid != target_pid {
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
        "12345\tmain\teditor\t0\t%0",
        12345,
        Some(("main", "editor", 0, "%0"))
    )]
    #[case::different_pid("12345\twork\tterminal\t1\t%5", 99999, None)]
    #[case::high_window_index(
        "54321\tsession\twindow\t99\t%123",
        54321,
        Some(("session", "window", 99, "%123"))
    )]
    #[case::session_with_spaces(
        "12345\tmy session\tmy window\t0\t%0",
        12345,
        Some(("my session", "my window", 0, "%0"))
    )]
    #[case::insufficient_parts("12345\tmain", 12345, None)]
    #[case::empty_line("", 12345, None)]
    fn test_parse_pane_line_by_pid(
        #[case] line: &str,
        #[case] target_pid: u32,
        #[case] expected: Option<(&str, &str, u32, &str)>,
    ) {
        let result = parse_pane_line_by_pid(line, target_pid);

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

    #[rstest]
    #[case::with_option_value(
        "main\t0\t1\t%5\tabc-123",
        Some(("main", 0, 1, "%5", "abc-123"))
    )]
    #[case::uuid_option(
        "work\t2\t0\t%10\t550e8400-e29b-41d4-a716-446655440000",
        Some(("work", 2, 0, "%10", "550e8400-e29b-41d4-a716-446655440000"))
    )]
    #[case::session_with_slash(
        "fohte/repo\t1\t2\t%3\txyz-456",
        Some(("fohte/repo", 1, 2, "%3", "xyz-456"))
    )]
    #[case::empty_option_value("main\t0\t1\t%5\t", None)]
    #[case::missing_option_field("main\t0\t1\t%5", None)]
    #[case::insufficient_parts("main\t0\t1", None)]
    #[case::empty_line("", None)]
    #[case::invalid_window_index("main\tabc\t1\t%5\toption", None)]
    #[case::invalid_pane_index("main\t0\tabc\t%5\toption", None)]
    fn test_parse_pane_with_option_line(
        #[case] line: &str,
        #[case] expected: Option<(&str, u32, u32, &str, &str)>,
    ) {
        let result = parse_pane_with_option_line(line);

        match expected {
            Some((session, window_idx, pane_idx, pane_id, option)) => {
                let info = result.expect("expected Some(PaneInfoWithOption)");
                assert_eq!(info.session_name, session);
                assert_eq!(info.window_index, window_idx);
                assert_eq!(info.pane_index, pane_idx);
                assert_eq!(info.pane_id, pane_id);
                assert_eq!(info.option_value, Some(option.to_string()));
            }
            None => assert!(result.is_none()),
        }
    }
}
