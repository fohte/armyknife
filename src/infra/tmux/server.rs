use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::time::Duration;

use crate::infra::external_tool::ExternalTool;
use crate::infra::process;

use super::{Result, TmuxError};

const TMUX_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs a tmux command against a recorded server socket.
///
/// Records predating socket tracking use the conventional default socket.
pub(crate) fn run_tmux_output_for_socket<S: AsRef<OsStr>>(
    socket_path: Option<&str>,
    args: &[S],
) -> Result<String> {
    let mut command = ExternalTool::Tmux.command();
    let mut command_args = socket_selector_args(socket_path);
    let mut display_args = command_args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    command_args.extend(args.iter().map(|arg| arg.as_ref().to_os_string()));
    display_args.extend(
        args.iter()
            .map(|arg| arg.as_ref().to_string_lossy().into_owned()),
    );
    command.args(&command_args);

    let output = process::run_with_timeout(command, TMUX_COMMAND_TIMEOUT)
        .map_err(|error| TmuxError::command_failed(&display_args, error.to_string(), None))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(TmuxError::command_failed(
            &display_args,
            "command exited with non-zero status",
            Some(stderr),
        ))
    }
}

fn socket_selector_args(socket_path: Option<&str>) -> Vec<OsString> {
    match socket_path {
        Some(path) => vec![OsString::from("-S"), OsString::from(path)],
        None => vec![OsString::from("-L"), OsString::from("default")],
    }
}

/// Returns the socket path of the server selected by the current environment.
pub fn current_socket_path() -> Option<String> {
    super::run_tmux_output(&["display-message", "-p", "#{socket_path}"])
        .ok()
        .filter(|path| !path.is_empty())
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

    for line in output.lines() {
        let Some((listed_pane_id, pane_pid)) = line.split_once('\t') else {
            if line == pane_id {
                return Err(TmuxError::Internal(format!(
                    "tmux returned no process ID for pane `{pane_id}`"
                )));
            }
            continue;
        };
        if listed_pane_id == pane_id {
            return pane_pid.parse::<u32>().map(Some).map_err(|_| {
                TmuxError::Internal(format!(
                    "tmux returned an invalid process ID for pane `{pane_id}`"
                ))
            });
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
}
