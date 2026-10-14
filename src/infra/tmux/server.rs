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
    let mut full_args = socket_selector_args(socket_path);
    full_args.extend(args.iter().map(|arg| arg.as_ref().to_os_string()));
    super::run_tmux_output(&full_args)
}

fn socket_selector_args(socket_path: Option<&str>) -> Vec<OsString> {
    match socket_path {
        Some(path) => vec![OsString::from("-S"), OsString::from(path)],
        None => vec![OsString::from("-L"), OsString::from("default")],
    }
}

/// Returns the socket path of the server selected by the current environment.
pub fn current_socket_path() -> Result<Option<String>> {
    let path = super::run_tmux_output(&["display-message", "-p", "#{socket_path}"])?;
    Ok((!path.is_empty()).then_some(path))
}

/// Lists pane IDs from the recorded server. An error means liveness is unknown.
pub fn list_all_pane_ids_for_socket(socket_path: Option<&str>) -> Result<HashSet<String>> {
    let output =
        run_tmux_output_for_socket(socket_path, &["list-panes", "-a", "-F", "#{pane_id}"])?;
    Ok(output.lines().map(str::to_owned).collect())
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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

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
