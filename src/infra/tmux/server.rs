use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::Path;

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

pub(crate) fn run_tmux_for_socket<S: AsRef<OsStr>>(
    socket_path: Option<&str>,
    args: &[S],
) -> Result<()> {
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
    InvalidPaneId,
}

fn parse_pane_ids(output: &str) -> std::result::Result<HashSet<String>, PaneIdsParseError> {
    let mut pane_ids = HashSet::new();

    for pane_id in output.lines() {
        let Some(suffix) = pane_id.strip_prefix('%') else {
            return Err(PaneIdsParseError::InvalidPaneId);
        };
        if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(PaneIdsParseError::InvalidPaneId);
        }
        pane_ids.insert(pane_id.to_owned());
    }

    if pane_ids.is_empty() {
        return Err(PaneIdsParseError::Empty);
    }

    Ok(pane_ids)
}

/// Lists pane IDs from the recorded server. An error means liveness is unknown.
///
/// Empty output is ambiguous because a successful command may omit pane IDs entirely.
pub fn list_all_pane_ids_for_socket(socket_path: Option<&str>) -> Result<HashSet<String>> {
    let output = super::run_tmux_output_untrimmed(&socket_command_args(
        socket_path,
        &["list-panes", "-a", "-F", "#{pane_id}"],
    ))?;
    parse_pane_ids(&output).map_err(|error| {
        let (reason, message) = match error {
            PaneIdsParseError::Empty => ("empty output", "tmux returned no pane IDs"),
            PaneIdsParseError::InvalidPaneId => {
                ("invalid pane ID", "tmux returned an invalid pane ID")
            }
        };
        tracing::warn!(
            ?socket_path,
            reason,
            "tmux list-panes output is untrusted; pane liveness is unknown"
        );
        TmuxError::Internal(message.to_owned())
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

fn capture_pane_for_socket_result_with(
    socket_path: Option<&str>,
    pane_id: &str,
    run_tmux_output: impl FnOnce(&[OsString]) -> super::Result<String>,
) -> super::Result<String> {
    run_tmux_output(&socket_command_args(
        socket_path,
        &["capture-pane", "-p", "-t", pane_id],
    ))
}

/// Captures the pane currently bound to a session, falling back to its stored pane ID.
pub(crate) fn capture_pane_for_session_for_socket_result(
    socket_path: Option<&str>,
    session_name: &str,
    session_id: &str,
    stored_pane_id: &str,
    option: &str,
    legacy_option: &str,
) -> super::Result<String> {
    capture_pane_for_session_for_socket_result_with(
        socket_path,
        session_name,
        session_id,
        stored_pane_id,
        option,
        legacy_option,
        super::run_tmux_output_untrimmed,
    )
}

fn capture_pane_for_session_for_socket_result_with(
    socket_path: Option<&str>,
    session_name: &str,
    session_id: &str,
    stored_pane_id: &str,
    option: &str,
    legacy_option: &str,
    mut run_tmux_output: impl FnMut(&[OsString]) -> super::Result<String>,
) -> super::Result<String> {
    let format = format!("#{{pane_id}}\t#{{session_name}}\t#{{{option}}}\t#{{{legacy_option}}}");
    let args = socket_command_args(socket_path, &["list-panes", "-a", "-F", format.as_str()]);
    let output = run_tmux_output(&args)?;
    let pane_id = find_session_pane_id(&output, session_name, session_id, stored_pane_id)
        .unwrap_or(stored_pane_id);

    capture_pane_for_socket_result_with(socket_path, pane_id, |args| run_tmux_output(args))
}

fn find_session_pane_id<'a>(
    output: &'a str,
    session_name: &str,
    session_id: &str,
    stored_pane_id: &'a str,
) -> Option<&'a str> {
    let mut stored_pane = None;
    for line in output.lines() {
        let mut fields = line.split('\t');
        let (Some(pane_id), Some(listed_session_name), Some(option_value)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let legacy_option_value = fields.next().unwrap_or("");
        let option_value = [option_value, legacy_option_value]
            .into_iter()
            .find(|value| !value.is_empty());

        if listed_session_name != session_name
            || !is_safe_pane_id(pane_id)
            || option_value != Some(session_id)
        {
            continue;
        }

        // Resume marks its replacement pane before sending the startup command.
        if pane_id != stored_pane_id {
            return Some(pane_id);
        }
        stored_pane = Some(pane_id);
    }

    stored_pane
}

/// Sends keys to a pane on its recorded server.
pub fn send_keys_for_socket(socket_path: Option<&str>, pane_id: &str, keys: &str) -> Result<()> {
    send_keys_sequence_for_socket(socket_path, pane_id, &[keys])
}

/// Sends a sequence of tmux keys in one command so concurrent actions cannot interleave it.
pub fn send_keys_sequence_for_socket(
    socket_path: Option<&str>,
    pane_id: &str,
    keys: &[&str],
) -> Result<()> {
    let mut args = vec!["send-keys", "-t", pane_id];
    args.extend_from_slice(keys);
    run_tmux_for_socket(socket_path, &args)
}

/// Sends keys only if the pane still runs the command observed by the caller.
pub fn send_keys_if_pane_command_matches_for_socket(
    socket_path: Option<&str>,
    pane_id: &str,
    expected_current_command: &str,
    keys: &[&str],
) -> Result<bool> {
    send_keys_if_pane_command_matches_with(
        socket_path,
        pane_id,
        expected_current_command,
        keys,
        |socket_path, args| run_tmux_output_for_socket(socket_path, &args),
    )
}

fn send_keys_if_pane_command_matches_with(
    socket_path: Option<&str>,
    pane_id: &str,
    expected_current_command: &str,
    keys: &[&str],
    run_command: impl FnOnce(Option<&str>, Vec<String>) -> Result<String>,
) -> Result<bool> {
    if !is_safe_pane_id(pane_id)
        || !is_safe_tmux_word(expected_current_command)
        || keys.is_empty()
        || keys.iter().any(|key| !is_safe_key(key))
    {
        return Err(TmuxError::Internal(
            "invalid pane command key sequence".to_string(),
        ));
    }

    let condition = format!("#{{==:#{{pane_current_command}},{expected_current_command}}}");
    let send_command = format!("send-keys -t {pane_id} {}", keys.join(" "));
    let reject_command = "display-message -p '__ARMYKNIFE_PANE_COMMAND_CHANGED__'";
    let args = vec![
        "if-shell".to_owned(),
        "-F".to_owned(),
        "-t".to_owned(),
        pane_id.to_owned(),
        condition,
        send_command,
        reject_command.to_owned(),
    ];
    let output = run_command(socket_path, args)?;

    Ok(output.trim() != "__ARMYKNIFE_PANE_COMMAND_CHANGED__")
}

fn is_safe_pane_id(pane_id: &str) -> bool {
    pane_id
        .strip_prefix('%')
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_safe_tmux_word(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn is_safe_key(key: &str) -> bool {
    matches!(key, "Down" | "Enter" | "Escape")
}

/// Focuses a pane on its recorded tmux server.
pub fn focus_pane_for_socket(socket_path: Option<&str>, pane_id: &str) -> Result<()> {
    let Some(socket_path) = socket_path else {
        return super::focus_pane(pane_id);
    };

    if super::current_socket_path().ok().flatten().as_deref() == Some(socket_path) {
        return super::focus_pane(pane_id);
    }

    let clients_output = run_tmux_output_for_socket(
        Some(socket_path),
        &["list-clients", "-F", "#{client_activity} #{client_tty}"],
    )?;
    if let Some(client_tty) = clients_output
        .lines()
        .filter_map(|line| {
            let (activity, tty) = line.split_once(' ')?;
            Some((activity.parse::<u64>().ok()?, tty))
        })
        .max_by_key(|(activity, _)| *activity)
        .map(|(_, tty)| tty)
    {
        run_tmux_for_socket(
            Some(socket_path),
            &["switch-client", "-c", client_tty, "-t", pane_id],
        )
    } else {
        run_tmux_for_socket(Some(socket_path), &["select-pane", "-t", pane_id])
    }
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
    cwd: &Path,
    command: &str,
) -> Result<()> {
    run_tmux_for_socket(socket_path, &respawn_pane_args(pane_id, cwd, command))
}

fn respawn_pane_args(pane_id: &str, cwd: &Path, command: &str) -> Vec<OsString> {
    [
        OsString::from("respawn-pane"),
        OsString::from("-k"),
        OsString::from("-c"),
        cwd.as_os_str().to_os_string(),
        OsString::from("-t"),
        OsString::from(pane_id),
        OsString::from(command),
    ]
    .into()
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

    #[test]
    fn plain_pane_capture_uses_the_recorded_socket_and_preserves_output() {
        let expected_output = ["", "fixture pane output", ""].join("\n");
        let mut actual_args = None;
        let actual_output = capture_pane_for_socket_result_with(
            Some("/tmp/armyknife-test/private.sock"),
            "%47",
            |args| {
                actual_args = Some(
                    args.iter()
                        .map(|arg| arg.to_string_lossy().into_owned())
                        .collect::<Vec<_>>(),
                );
                Ok(expected_output.clone())
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (actual_output, actual_args),
            (
                Ok(expected_output),
                Some(vec![
                    "-S".to_owned(),
                    "/tmp/armyknife-test/private.sock".to_owned(),
                    "capture-pane".to_owned(),
                    "-p".to_owned(),
                    "-t".to_owned(),
                    "%47".to_owned(),
                ]),
            ),
        );
    }

    #[test]
    fn session_pane_capture_prefers_a_replacement_pane_bound_to_the_session() {
        let pane_listing = indoc! {"
            %17\tfixture-tmux\tfixture-session\t
            %29\tfixture-tmux\tfixture-session\t
            %31\tother-tmux\tfixture-session\t
            %33\tfixture-tmux\tother-session\t
        "}
        .to_string();
        let expected_output = ["", "fixture startup error", ""].join("\n");
        let mut actual_calls = Vec::new();
        let actual_output = capture_pane_for_session_for_socket_result_with(
            Some("/fixture/tmux.sock"),
            "fixture-tmux",
            "fixture-session",
            "%17",
            "@fixture-session-id",
            "@fixture-session-id-legacy",
            |args| {
                let args = args
                    .iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                actual_calls.push(args.clone());
                if args.get(2).map(String::as_str) == Some("list-panes") {
                    Ok(pane_listing.clone())
                } else {
                    Ok(expected_output.clone())
                }
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (actual_output, actual_calls),
            (
                Ok(expected_output),
                vec![
                    vec![
                        "-S",
                        "/fixture/tmux.sock",
                        "list-panes",
                        "-a",
                        "-F",
                        "#{pane_id}\t#{session_name}\t#{@fixture-session-id}\t#{@fixture-session-id-legacy}",
                    ],
                    vec![
                        "-S",
                        "/fixture/tmux.sock",
                        "capture-pane",
                        "-p",
                        "-t",
                        "%29",
                    ],
                ]
                .into_iter()
                .map(|args| args.into_iter().map(str::to_owned).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            ),
        );
    }

    #[test]
    fn respawn_pane_uses_the_recorded_working_directory() {
        assert_eq!(
            respawn_pane_args("%fixture", Path::new("/fixture/worktree"), "a agent resume"),
            vec![
                "respawn-pane",
                "-k",
                "-c",
                "/fixture/worktree",
                "-t",
                "%fixture",
                "a agent resume",
            ],
        );
    }

    #[test]
    fn guarded_key_sender_runs_fixed_keys_through_the_pane_check() {
        let mut invocation = None;
        let result = send_keys_if_pane_command_matches_with(
            Some("/tmp/armyknife-test/private.sock"),
            "%47",
            "claude",
            &["Down", "Enter", "Escape"],
            |socket_path, args| {
                invocation = Some((socket_path.map(str::to_owned), args));
                Ok(String::new())
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, invocation),
            (
                Ok(true),
                Some((
                    Some("/tmp/armyknife-test/private.sock".to_owned()),
                    vec![
                        "if-shell".to_owned(),
                        "-F".to_owned(),
                        "-t".to_owned(),
                        "%47".to_owned(),
                        "#{==:#{pane_current_command},claude}".to_owned(),
                        "send-keys -t %47 Down Enter Escape".to_owned(),
                        "display-message -p '__ARMYKNIFE_PANE_COMMAND_CHANGED__'".to_owned(),
                    ],
                )),
            ),
        );
    }

    #[rstest]
    #[case::numeric_key(vec!["1"])]
    #[case::untrusted_key(vec!["Enter; display-message injected"])]
    fn guarded_key_sender_rejects_unlisted_keys_before_running_tmux(#[case] keys: Vec<&str>) {
        let mut invoked = false;
        let result = send_keys_if_pane_command_matches_with(
            Some("/tmp/armyknife-test/private.sock"),
            "%47",
            "claude",
            &keys,
            |_, _| {
                invoked = true;
                Ok(String::new())
            },
        )
        .map(|_| "sent")
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, invoked),
            (Err("invalid pane command key sequence".to_owned()), false),
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
    #[case::non_pane_output("tmux unavailable\n", Err(PaneIdsParseError::InvalidPaneId))]
    #[case::malformed_pane_id(
        indoc! {"
            %47
            %48x
        "},
        Err(PaneIdsParseError::InvalidPaneId)
    )]
    #[case::surrounding_whitespace_is_invalid(" %47 \n", Err(PaneIdsParseError::InvalidPaneId))]
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
