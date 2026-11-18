use super::{TmuxCommand, background_pane_prefix};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TmuxServer<'a> {
    Current,
    Socket(Option<&'a str>),
}

trait TmuxCommandRunner {
    fn output(&self, server: TmuxServer<'_>, args: &[&str]) -> crate::infra::tmux::Result<String>;
    fn run(&self, server: TmuxServer<'_>, args: &[&str]) -> crate::infra::tmux::Result<()>;
}

struct LiveTmuxCommandRunner;

impl TmuxCommandRunner for LiveTmuxCommandRunner {
    fn output(&self, server: TmuxServer<'_>, args: &[&str]) -> crate::infra::tmux::Result<String> {
        match server {
            TmuxServer::Current => crate::infra::tmux::run_tmux_output(args),
            TmuxServer::Socket(socket_path) => {
                crate::infra::tmux::run_tmux_output_for_socket(socket_path, args)
            }
        }
    }

    fn run(&self, server: TmuxServer<'_>, args: &[&str]) -> crate::infra::tmux::Result<()> {
        match server {
            TmuxServer::Current => crate::infra::tmux::run_tmux(args),
            TmuxServer::Socket(socket_path) => {
                crate::infra::tmux::run_tmux_for_socket(socket_path, args)
            }
        }
    }
}

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

/// Executes a layout and captures the stable window ID for Codex pane binding.
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
    execute_layout_with(
        commands,
        session,
        window_name,
        background,
        TmuxServer::Current,
        &LiveTmuxCommandRunner,
    )
}

pub(super) fn execute_layout_for_socket(
    commands: &[TmuxCommand],
    session: &str,
    window_name: &str,
    background: bool,
    socket_path: Option<&str>,
) -> crate::infra::tmux::Result<String> {
    execute_layout_with(
        commands,
        session,
        window_name,
        background,
        TmuxServer::Socket(socket_path),
        &LiveTmuxCommandRunner,
    )
}

fn execute_layout_with(
    commands: &[TmuxCommand],
    session: &str,
    window_name: &str,
    background: bool,
    server: TmuxServer<'_>,
    runner: &impl TmuxCommandRunner,
) -> crate::infra::tmux::Result<String> {
    let new_window_idx = find_new_window_index(commands).ok_or_else(|| {
        crate::infra::tmux::TmuxError::Internal(
            "new-window command not found in layout".to_string(),
        )
    })?;

    if !background {
        let commands = with_window_id_capture(commands, new_window_idx);
        return runner.output(server, &flatten_commands(&commands));
    }

    let setup = &commands[..=new_window_idx];
    let rest = &commands[new_window_idx + 1..];

    let mut setup_args = flatten_commands(setup);
    setup_args.extend(["-P", "-F", "#{window_id}"]);
    let window_id = runner.output(server, &setup_args)?;

    let old_prefix = background_pane_prefix(session, window_name);
    let new_prefix = format!("{window_id}.");
    let commands = rewrite_pane_targets(rest, &old_prefix, &new_prefix);
    runner.run(server, &flatten_commands(&commands))?;
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::{Value, json};

    use super::{TmuxCommandRunner, TmuxServer, execute_layout_with};
    use crate::infra::tmux::layout::TmuxCommand;

    #[derive(Default)]
    struct FakeTmuxCommandRunner {
        calls: Mutex<Vec<Value>>,
    }

    impl TmuxCommandRunner for FakeTmuxCommandRunner {
        fn output(
            &self,
            server: TmuxServer<'_>,
            args: &[&str],
        ) -> crate::infra::tmux::Result<String> {
            self.calls.lock().unwrap().push(json!({
                "kind": "output",
                "server": server_snapshot(server),
                "args": args,
            }));
            Ok("@fixture-window".to_string())
        }

        fn run(&self, server: TmuxServer<'_>, args: &[&str]) -> crate::infra::tmux::Result<()> {
            self.calls.lock().unwrap().push(json!({
                "kind": "run",
                "server": server_snapshot(server),
                "args": args,
            }));
            Ok(())
        }
    }

    fn server_snapshot(server: TmuxServer<'_>) -> Value {
        match server {
            TmuxServer::Current => json!({ "kind": "current" }),
            TmuxServer::Socket(socket_path) => json!({
                "kind": "socket",
                "path": socket_path,
            }),
        }
    }

    #[test]
    fn socket_layout_executes_new_window_then_targets_its_captured_id() {
        let commands = vec![
            TmuxCommand {
                args: vec![
                    "new-window".to_string(),
                    "-d".to_string(),
                    "-t".to_string(),
                    "fixture-session".to_string(),
                    "-n".to_string(),
                    "fixture-window".to_string(),
                ],
            },
            TmuxCommand {
                args: vec![
                    "split-window".to_string(),
                    "-h".to_string(),
                    "-t".to_string(),
                    "fixture-session:=fixture-window.1".to_string(),
                ],
            },
            TmuxCommand {
                args: vec![
                    "send-keys".to_string(),
                    "-t".to_string(),
                    "fixture-session:=fixture-window.2".to_string(),
                    "C-m".to_string(),
                ],
            },
        ];
        let runner = FakeTmuxCommandRunner::default();

        let result = execute_layout_with(
            &commands,
            "fixture-session",
            "fixture-window",
            true,
            TmuxServer::Socket(Some("/fixture/tmux.sock")),
            &runner,
        );

        assert_eq!(
            (result.unwrap(), runner.calls.lock().unwrap().clone()),
            (
                "@fixture-window".to_string(),
                vec![
                    json!({
                        "kind": "output",
                        "server": { "kind": "socket", "path": "/fixture/tmux.sock" },
                        "args": [
                            "new-window", "-d", "-t", "fixture-session", "-n", "fixture-window",
                            "-P", "-F", "#{window_id}",
                        ],
                    }),
                    json!({
                        "kind": "run",
                        "server": { "kind": "socket", "path": "/fixture/tmux.sock" },
                        "args": [
                            "split-window", "-h", "-t", "@fixture-window.1", ";",
                            "send-keys", "-t", "@fixture-window.2", "C-m",
                        ],
                    }),
                ],
            ),
        );
    }
}
