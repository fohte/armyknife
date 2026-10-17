use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};

use super::{Result, TmuxError};

#[derive(Default)]
pub(crate) struct PaneLiveness {
    panes_by_socket: HashMap<Option<String>, Option<HashSet<String>>>,
}

impl PaneLiveness {
    pub(crate) fn cache_socket<E>(
        &mut self,
        socket_path: Option<&str>,
        list_panes: impl FnOnce(Option<&str>) -> std::result::Result<HashSet<String>, E>,
    ) {
        let key = socket_path.map(str::to_owned);
        self.panes_by_socket
            .entry(key)
            .or_insert_with(|| list_panes(socket_path).ok());
    }

    pub(crate) fn is_alive(&self, socket_path: Option<&str>, pane_id: &str) -> Option<bool> {
        self.panes_by_socket
            .get(&socket_path.map(str::to_owned))
            .and_then(Option::as_ref)
            .map(|panes| panes.contains(pane_id))
    }
}

/// Runs a tmux command against a recorded server socket.
///
/// Records predating socket tracking use the conventional default socket.
pub(crate) fn run_tmux_output_for_socket<S: AsRef<OsStr>>(
    socket_path: Option<&str>,
    args: &[S],
) -> Result<String> {
    super::run_tmux_output(&socket_command_args(socket_path, args))
}

fn socket_selector_args(socket_path: Option<&str>) -> Vec<OsString> {
    match socket_path {
        Some(path) => vec![OsString::from("-S"), OsString::from(path)],
        None => vec![OsString::from("-L"), OsString::from("default")],
    }
}

fn socket_command_args<S: AsRef<OsStr>>(socket_path: Option<&str>, args: &[S]) -> Vec<OsString> {
    let mut full_args = socket_selector_args(socket_path);
    full_args.extend(args.iter().map(|arg| arg.as_ref().to_os_string()));
    full_args
}

fn run_tmux_for_socket<S: AsRef<OsStr>>(socket_path: Option<&str>, args: &[S]) -> Result<()> {
    run_tmux_output_for_socket(socket_path, args).map(|_| ())
}

/// Returns the socket path of the server selected by the current environment.
pub fn current_socket_path() -> Result<Option<String>> {
    let path = super::run_tmux_output(&["display-message", "-p", "#{socket_path}"])?;
    Ok((!path.is_empty()).then_some(path))
}

#[derive(Debug, PartialEq, Eq)]
enum PaneIdsParseError {
    Empty,
    InvalidPaneId(String),
}

fn parse_pane_ids(output: &str) -> std::result::Result<HashSet<String>, PaneIdsParseError> {
    let mut pane_ids = HashSet::new();

    for pane_id in output.lines() {
        let Some(suffix) = pane_id.strip_prefix('%') else {
            return Err(PaneIdsParseError::InvalidPaneId(pane_id.to_owned()));
        };
        if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(PaneIdsParseError::InvalidPaneId(pane_id.to_owned()));
        }
        pane_ids.insert(pane_id.to_owned());
    }

    if pane_ids.is_empty() {
        return Err(PaneIdsParseError::Empty);
    }

    Ok(pane_ids)
}

/// Lists pane IDs from the recorded server. An error means liveness is unknown.
pub fn list_all_pane_ids_for_socket(socket_path: Option<&str>) -> Result<HashSet<String>> {
    let output = super::run_tmux_output_untrimmed(&socket_command_args(
        socket_path,
        &["list-panes", "-a", "-F", "#{pane_id}"],
    ))?;
    parse_pane_ids(&output).map_err(|error| match error {
        PaneIdsParseError::Empty => TmuxError::Internal("tmux returned no pane IDs".to_owned()),
        PaneIdsParseError::InvalidPaneId(_) => {
            TmuxError::Internal("tmux returned an invalid pane ID".to_owned())
        }
    })
}

/// Returns whether a pane exists in its recorded server.
pub fn is_pane_alive_for_socket(socket_path: Option<&str>, pane_id: &str) -> Result<bool> {
    Ok(list_all_pane_ids_for_socket(socket_path)?.contains(pane_id))
}

/// Resolves a pane's foreground process ID without treating a server error as a missing pane.
pub fn get_pane_pid_for_socket(socket_path: Option<&str>, pane_id: &str) -> Result<Option<u32>> {
    let output = run_tmux_output_for_socket(
        socket_path,
        &["list-panes", "-a", "-F", "#{pane_id}\t#{pane_pid}"],
    )?;

    parse_pane_pid_output(&output, pane_id).map_err(|error| match error {
        PanePidParseError::MissingPid => {
            TmuxError::Internal(format!("tmux returned no process ID for pane `{pane_id}`"))
        }
        PanePidParseError::InvalidPid => TmuxError::Internal(format!(
            "tmux returned an invalid process ID for pane `{pane_id}`"
        )),
    })
}

#[derive(Debug, PartialEq, Eq)]
enum PanePidParseError {
    MissingPid,
    InvalidPid,
}

fn parse_pane_pid_output(
    output: &str,
    pane_id: &str,
) -> std::result::Result<Option<u32>, PanePidParseError> {
    for line in output.lines() {
        let Some((listed_pane_id, pane_pid)) = line.split_once('\t') else {
            if line == pane_id {
                return Err(PanePidParseError::MissingPid);
            }
            continue;
        };
        if listed_pane_id == pane_id {
            return pane_pid
                .parse::<u32>()
                .map(Some)
                .map_err(|_| PanePidParseError::InvalidPid);
        }
    }

    Ok(None)
}

/// Sends SIGTERM to the process running in a pane on its recorded server.
pub fn send_sigterm_to_pane_for_socket(socket_path: Option<&str>, pane_id: &str) {
    if let Ok(Some(pid)) = get_pane_pid_for_socket(socket_path, pane_id) {
        // SAFETY: libc::kill with SIGTERM is safe for any valid PID.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
}

/// Reads a pane-scoped user option from its recorded server.
pub fn get_pane_option_for_socket(
    socket_path: Option<&str>,
    pane_id: &str,
    option: &str,
) -> Option<String> {
    let output = run_tmux_output_for_socket(
        socket_path,
        &["show-options", "-p", "-t", pane_id, "-v", option],
    )
    .ok()?;
    (!output.is_empty()).then_some(output)
}

/// Sets a pane-scoped user option on its recorded server.
pub fn set_pane_option_for_socket(
    socket_path: Option<&str>,
    pane_id: &str,
    option: &str,
    value: &str,
) -> Result<()> {
    run_tmux_for_socket(
        socket_path,
        &["set-option", "-p", "-t", pane_id, option, value],
    )
}

/// Captures a pane from its recorded server without ANSI styling.
pub fn capture_pane_for_socket(socket_path: Option<&str>, pane_id: &str) -> Option<String> {
    run_tmux_output_for_socket(socket_path, &["capture-pane", "-p", "-t", pane_id]).ok()
}

/// Captures a pane from its recorded server with ANSI styling.
pub fn capture_pane_with_escapes_for_socket(
    socket_path: Option<&str>,
    pane_id: &str,
) -> Option<String> {
    run_tmux_output_for_socket(socket_path, &["capture-pane", "-p", "-e", "-t", pane_id]).ok()
}

/// Sends keys to a pane on its recorded server.
pub fn send_keys_for_socket(socket_path: Option<&str>, pane_id: &str, keys: &str) -> Result<()> {
    run_tmux_for_socket(socket_path, &["send-keys", "-t", pane_id, keys])
}

/// Kills a pane on its recorded server.
pub fn kill_pane_for_socket(socket_path: Option<&str>, pane_id: &str) -> Result<()> {
    run_tmux_for_socket(socket_path, &["kill-pane", "-t", pane_id])
}

/// Returns a pane's current command from its recorded server.
pub fn get_pane_current_command_for_socket(
    socket_path: Option<&str>,
    pane_id: &str,
) -> Option<String> {
    let output = run_tmux_output_for_socket(
        socket_path,
        &[
            "display-message",
            "-p",
            "-t",
            pane_id,
            "#{pane_current_command}",
        ],
    )
    .ok()?;
    (!output.is_empty()).then_some(output)
}

/// Restarts a pane on its recorded server with `command`.
pub fn respawn_pane_for_socket(
    socket_path: Option<&str>,
    pane_id: &str,
    command: &str,
) -> Result<()> {
    run_tmux_for_socket(socket_path, &["respawn-pane", "-k", "-t", pane_id, command])
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use indoc::indoc;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::legacy_targets_default_server(None, vec!["-L", "default"])]
    #[case::recorded_socket_is_explicit(
        Some("/tmp/armyknife-test/private.sock"),
        vec!["-S", "/tmp/armyknife-test/private.sock"]
    )]
    fn selects_server_socket(#[case] socket_path: Option<&str>, #[case] expected: Vec<&str>) {
        assert_eq!(
            socket_selector_args(socket_path),
            expected.into_iter().map(OsString::from).collect::<Vec<_>>(),
        );
    }

    #[rstest]
    #[case::legacy_commands_use_default_server(
        None,
        vec!["list-panes", "-a", "-F", "#{pane_id}"],
        vec!["-L", "default", "list-panes", "-a", "-F", "#{pane_id}"]
    )]
    #[case::recorded_commands_use_recorded_server(
        Some("/tmp/armyknife-test/private.sock"),
        vec!["kill-pane", "-t", "%81234"],
        vec!["-S", "/tmp/armyknife-test/private.sock", "kill-pane", "-t", "%81234"]
    )]
    fn prepends_server_selection_to_command(
        #[case] socket_path: Option<&str>,
        #[case] args: Vec<&str>,
        #[case] expected: Vec<&str>,
    ) {
        assert_eq!(
            socket_command_args(socket_path, &args),
            expected.into_iter().map(OsString::from).collect::<Vec<_>>(),
        );
    }

    #[rstest]
    #[case::single_pane("%47\n", Ok(HashSet::from(["%47".to_owned()])))]
    #[case::multiple_panes(
        indoc! {"
            %47
            %48
        "},
        Ok(HashSet::from(["%47".to_owned(), "%48".to_owned()]))
    )]
    #[case::empty_output("", Err(PaneIdsParseError::Empty))]
    #[case::non_pane_output(
        "tmux unavailable\n",
        Err(PaneIdsParseError::InvalidPaneId("tmux unavailable".to_owned()))
    )]
    #[case::malformed_pane_id(
        indoc! {"
            %47
            %48x
        "},
        Err(PaneIdsParseError::InvalidPaneId("%48x".to_owned()))
    )]
    #[case::surrounding_whitespace_is_invalid(
        " %47 \n",
        Err(PaneIdsParseError::InvalidPaneId(" %47 ".to_owned()))
    )]
    fn parses_pane_ids(
        #[case] output: &str,
        #[case] expected: std::result::Result<HashSet<String>, PaneIdsParseError>,
    ) {
        assert_eq!(parse_pane_ids(output), expected);
    }

    #[rstest]
    #[case::matches_exact_pane_id("%81234\t7101\n%812345\t7102", "%812345", Ok(Some(7102)))]
    #[case::missing_pane("%81234\t7101\n%812345\t7102", "%812346", Ok(None))]
    #[case::missing_pid("%812345", "%812345", Err(PanePidParseError::MissingPid))]
    #[case::invalid_pid("%812345\tnot-a-pid", "%812345", Err(PanePidParseError::InvalidPid))]
    fn parses_pane_pid_by_exact_id(
        #[case] output: &str,
        #[case] pane_id: &str,
        #[case] expected: std::result::Result<Option<u32>, PanePidParseError>,
    ) {
        assert_eq!(parse_pane_pid_output(output, pane_id), expected);
    }
}
