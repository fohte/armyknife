use super::{TmuxCommand, background_pane_prefix};

/// Flattens a sequence of `TmuxCommand` into a single arg list joined by `;`,
/// the wire format tmux uses to chain multiple commands in one invocation.
pub(super) fn flatten_commands(commands: &[TmuxCommand]) -> Vec<&str> {
    let mut args: Vec<&str> = Vec::new();
    for (i, cmd) in commands.iter().enumerate() {
        if i > 0 {
            args.push(";");
        }
        for arg in &cmd.args {
            args.push(arg);
        }
    }
    args
}

/// Executes a layout and captures the stable window ID for launch fallback.
/// Background layouts run `new-window` first, then rewrite the remaining
/// `{session}:={window_name}.` pane targets to `{window_id}.` before running
/// them. Foreground layouts keep the existing single tmux command sequence.
///
/// This indirection exists because tmux's target parser splits on `.` to
/// separate window from pane, so a session-qualified target is ambiguous when
/// the window name itself contains a `.` (e.g. a branch name like
/// `copier-update/v0.8.13` becomes window name `copier-update-v0.8.13`).
/// Window IDs (e.g. `@42`) never contain `.`, so targeting by ID sidesteps the
/// ambiguity entirely.
pub(super) fn execute_layout(
    commands: &[TmuxCommand],
    session: &str,
    window_name: &str,
    background: bool,
) -> crate::infra::tmux::Result<String> {
    let new_window_idx = find_new_window_index(commands).ok_or_else(|| {
        crate::infra::tmux::TmuxError::Internal(
            "new-window command not found in layout".to_string(),
        )
    })?;

    if !background {
        let commands = with_window_id_capture(commands, new_window_idx);
        return crate::infra::tmux::run_tmux_output(&flatten_commands(&commands));
    }

    let setup = &commands[..=new_window_idx];
    let rest = &commands[new_window_idx + 1..];

    let mut setup_args = flatten_commands(setup);
    setup_args.extend(["-P", "-F", "#{window_id}"]);
    let window_id = crate::infra::tmux::run_tmux_output(&setup_args)?;

    let old_prefix = background_pane_prefix(session, window_name);
    let new_prefix = format!("{window_id}.");
    execute_commands(&rewrite_pane_targets(rest, &old_prefix, &new_prefix))?;
    Ok(window_id)
}

pub(super) fn with_window_id_capture(
    commands: &[TmuxCommand],
    new_window_idx: usize,
) -> Vec<TmuxCommand> {
    let mut commands = commands.to_vec();
    commands[new_window_idx].args.extend([
        "-P".to_string(),
        "-F".to_string(),
        "#{window_id}".to_string(),
    ]);
    commands
}

/// Finds the index of the `new-window` command in a layout's command list.
pub(super) fn find_new_window_index(commands: &[TmuxCommand]) -> Option<usize> {
    commands
        .iter()
        .position(|cmd| cmd.args.first().map(String::as_str) == Some("new-window"))
}

/// Rewrites pane-targeting command args from `{old_prefix}{pane}` to
/// `{new_prefix}{pane}`.
pub(super) fn rewrite_pane_targets(
    commands: &[TmuxCommand],
    old_prefix: &str,
    new_prefix: &str,
) -> Vec<TmuxCommand> {
    commands
        .iter()
        .map(|cmd| TmuxCommand {
            args: cmd
                .args
                .iter()
                .map(|arg| match arg.strip_prefix(old_prefix) {
                    Some(rest) => format!("{new_prefix}{rest}"),
                    None => arg.clone(),
                })
                .collect(),
        })
        .collect()
}

/// Execute a sequence of TmuxCommand by chaining them with ";".
pub(super) fn execute_commands(commands: &[TmuxCommand]) -> crate::infra::tmux::Result<()> {
    crate::infra::tmux::run_tmux(&flatten_commands(commands))
}
