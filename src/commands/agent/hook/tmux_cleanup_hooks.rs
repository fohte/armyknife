use std::env;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Context;

use crate::commands::agent::store;
use crate::commands::agent::types::{HookEvent, TmuxInfo};
use crate::infra::tmux;

const HOOK_INDEX: usize = 1000;
const TMUX_HOOKS: [&str; 6] = [
    "pane-exited",
    "pane-died",
    "after-kill-pane",
    "window-unlinked",
    "after-kill-window",
    "session-closed",
];
const CLEANUP_ENVIRONMENT: [&str; 6] = [
    "PATH",
    "HOME",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "CODEX_HOME",
];

#[derive(Clone, Debug, PartialEq, Eq)]
enum TmuxTarget {
    Current,
    Recorded(Option<String>),
}

pub(super) fn should_configure(event: HookEvent, inside_tmux: bool, pane_resolved: bool) -> bool {
    event == HookEvent::SessionStart && (inside_tmux || pane_resolved)
}

pub(super) fn configure_for_session_start(session_id: &str) {
    on_session_start_with(spawn_startup_cleanup, || {
        configure_tmux_hooks_for_session_start(session_id)
    });
}

fn on_session_start_with(
    start_cleanup: impl FnOnce() -> anyhow::Result<()>,
    configure_hooks: impl FnOnce(),
) {
    if let Err(error) = start_cleanup() {
        tracing::warn!(
            event = "agent.hook.tmux_cleanup_hooks.startup_cleanup_failed",
            error = %error,
            "failed to start stale session cleanup at SessionStart"
        );
    }
    configure_hooks();
}

fn spawn_startup_cleanup() -> anyhow::Result<()> {
    let executable = env::current_exe().context("failed to resolve the armyknife executable")?;
    startup_cleanup_command(&executable)
        .spawn()
        .context("failed to start stale session cleanup")?;
    Ok(())
}

fn startup_cleanup_command(executable: &Path) -> Command {
    let mut command = crate::shared::command::new(executable);
    command
        .args(["agent", "cleanup-stale-sessions"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn configure_tmux_hooks_for_session_start(session_id: &str) {
    let result = configure_for_session_start_with(
        session_id,
        tmux::in_tmux(),
        |session_id| Ok(store::load_session(session_id)?.and_then(|session| session.tmux_info)),
        configure_from_current_process,
    );
    if let Err(error) = result {
        tracing::warn!(
            event = "agent.hook.tmux_cleanup_hooks.failed",
            error = %error,
            "failed to configure tmux cleanup hooks"
        );
    }
}

fn configure_for_session_start_with(
    session_id: &str,
    inside_tmux: bool,
    load_tmux_info: impl FnOnce(&str) -> anyhow::Result<Option<TmuxInfo>>,
    configure_target: impl FnOnce(TmuxTarget) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let tmux_info = match load_tmux_info(session_id) {
        Ok(tmux_info) => tmux_info,
        Err(error) => {
            tracing::warn!(
                event = "agent.hook.tmux_cleanup_hooks.session_lookup_failed",
                session_id,
                error = %error,
                "failed to load session for tmux cleanup hook setup"
            );
            None
        }
    };
    if !should_configure(HookEvent::SessionStart, inside_tmux, tmux_info.is_some()) {
        return Ok(());
    }

    let target = tmux_info.map_or(TmuxTarget::Current, |info| {
        TmuxTarget::Recorded(info.socket_path)
    });
    configure_target(target)
}

fn configure_from_current_process(target: TmuxTarget) -> anyhow::Result<()> {
    let executable = env::current_exe().context("failed to resolve the armyknife executable")?;
    let executable = executable
        .to_str()
        .context("armyknife executable path is not valid UTF-8")?;
    let environment = CLEANUP_ENVIRONMENT
        .iter()
        .filter_map(|name| env::var(name).ok().map(|value| (*name, value)))
        .collect::<Vec<_>>();
    let environment = environment
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect::<Vec<_>>();

    configure(executable, &environment, target, run_tmux_target)?;
    Ok(())
}

fn configure(
    executable: &str,
    environment: &[(&str, &str)],
    target: TmuxTarget,
    mut run_tmux: impl FnMut(&TmuxTarget, &[&str]) -> tmux::Result<()>,
) -> tmux::Result<()> {
    let command = cleanup_command(executable, environment);
    let mut args = Vec::with_capacity(TMUX_HOOKS.len() * 5);
    for (index, hook) in TMUX_HOOKS.iter().enumerate() {
        if index > 0 {
            args.push(";".to_string());
        }
        let indexed_hook = format!("{hook}[{HOOK_INDEX}]");
        args.extend([
            "set-hook".to_string(),
            "-g".to_string(),
            indexed_hook,
            command.clone(),
        ]);
    }

    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_tmux(&target, &args)
}

fn run_tmux_target(target: &TmuxTarget, args: &[&str]) -> tmux::Result<()> {
    match target {
        TmuxTarget::Current => tmux::run_tmux(args),
        TmuxTarget::Recorded(socket_path) => {
            tmux::run_tmux_for_socket(socket_path.as_deref(), args)
        }
    }
}

fn cleanup_command(executable: &str, environment: &[(&str, &str)]) -> String {
    let assignments = environment
        .iter()
        .map(|(name, value)| format!("{name}={}", shell_quote(value)))
        .collect::<Vec<_>>()
        .join(" ");
    let executable = shell_quote(executable);
    let shell_command = if assignments.is_empty() {
        format!("{executable} agent cleanup-stale-sessions")
    } else {
        format!("{assignments} {executable} agent cleanup-stale-sessions")
    };

    format!(
        "run-shell -b {}",
        tmux_double_quote(&format!("{shell_command} >/dev/null 2>&1"))
    )
}

fn shell_quote(value: &str) -> String {
    shlex::try_quote(value)
        .map(|quoted| quoted.into_owned())
        .unwrap_or_else(|_| format!("'{}'", value.replace('\'', "'\\''")))
}

fn tmux_double_quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('#', "##")
    )
}

#[cfg(test)]
mod tests {
    use super::{
        TmuxTarget, cleanup_command, configure, configure_for_session_start_with,
        on_session_start_with, should_configure, startup_cleanup_command,
    };
    use crate::commands::agent::types::HookEvent;

    #[test]
    fn configures_session_start_inside_tmux_or_with_a_resolved_pane() {
        assert_eq!(
            [
                should_configure(HookEvent::SessionStart, true, false),
                should_configure(HookEvent::SessionStart, false, true),
                should_configure(HookEvent::SessionStart, false, false),
                should_configure(HookEvent::UserPromptSubmit, true, true),
            ],
            [true, true, false, false],
        );
    }

    #[test]
    fn session_start_starts_cleanup_before_hook_setup_even_when_spawning_fails() {
        let events = std::cell::RefCell::new(Vec::new());

        on_session_start_with(
            || {
                events.borrow_mut().push("cleanup");
                Err(anyhow::anyhow!("fixture spawn failure"))
            },
            || events.borrow_mut().push("configure hooks"),
        );

        assert_eq!(events.into_inner(), vec!["cleanup", "configure hooks"],);
    }

    #[test]
    fn startup_cleanup_uses_the_existing_subcommand() {
        let command = startup_cleanup_command(std::path::Path::new("/fixture/bin/a"));

        assert_eq!(
            (
                command.get_program().to_os_string(),
                command
                    .get_args()
                    .map(std::ffi::OsStr::to_os_string)
                    .collect::<Vec<_>>(),
            ),
            (
                std::ffi::OsString::from("/fixture/bin/a"),
                vec![
                    std::ffi::OsString::from("agent"),
                    std::ffi::OsString::from("cleanup-stale-sessions"),
                ],
            ),
        );
    }

    #[test]
    fn interactive_session_start_uses_the_current_tmux_server_without_a_recorded_binding() {
        let mut actual_target = None;

        configure_for_session_start_with(
            "fixture-session",
            true,
            |_| Ok(None),
            |target| {
                actual_target = Some(target);
                Ok(())
            },
        )
        .expect("current tmux server should be selected");

        assert_eq!(actual_target, Some(TmuxTarget::Current));
    }

    #[test]
    fn daemon_session_start_configures_each_global_hook_on_the_recorded_socket() {
        let tmux_info = crate::commands::agent::types::TmuxInfo {
            session_name: "fixture-session".to_string(),
            window_name: "fixture-window".to_string(),
            window_index: 0,
            pane_id: "%73".to_string(),
            socket_path: Some("/fixture/tmux.sock".to_string()),
        };
        let mut calls = Vec::new();

        configure_for_session_start_with(
            "fixture-session",
            false,
            |_| Ok(Some(tmux_info)),
            |target| {
                configure(
                    "/fixture/bin/a",
                    &[
                        ("PATH", "/fixture/bin:/usr/bin"),
                        ("XDG_CACHE_HOME", "/fixture/cache"),
                    ],
                    target,
                    |target, args| {
                        calls.push((
                            target.clone(),
                            args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>(),
                        ));
                        Ok(())
                    },
                )?;
                Ok(())
            },
        )
        .expect("tmux hook setup should succeed");

        let expected_args = [
            "set-hook",
            "-g",
            "pane-exited[1000]",
            "run-shell -b \"PATH=/fixture/bin:/usr/bin XDG_CACHE_HOME=/fixture/cache /fixture/bin/a agent cleanup-stale-sessions >/dev/null 2>&1\"",
            ";",
            "set-hook",
            "-g",
            "pane-died[1000]",
            "run-shell -b \"PATH=/fixture/bin:/usr/bin XDG_CACHE_HOME=/fixture/cache /fixture/bin/a agent cleanup-stale-sessions >/dev/null 2>&1\"",
            ";",
            "set-hook",
            "-g",
            "after-kill-pane[1000]",
            "run-shell -b \"PATH=/fixture/bin:/usr/bin XDG_CACHE_HOME=/fixture/cache /fixture/bin/a agent cleanup-stale-sessions >/dev/null 2>&1\"",
            ";",
            "set-hook",
            "-g",
            "window-unlinked[1000]",
            "run-shell -b \"PATH=/fixture/bin:/usr/bin XDG_CACHE_HOME=/fixture/cache /fixture/bin/a agent cleanup-stale-sessions >/dev/null 2>&1\"",
            ";",
            "set-hook",
            "-g",
            "after-kill-window[1000]",
            "run-shell -b \"PATH=/fixture/bin:/usr/bin XDG_CACHE_HOME=/fixture/cache /fixture/bin/a agent cleanup-stale-sessions >/dev/null 2>&1\"",
            ";",
            "set-hook",
            "-g",
            "session-closed[1000]",
            "run-shell -b \"PATH=/fixture/bin:/usr/bin XDG_CACHE_HOME=/fixture/cache /fixture/bin/a agent cleanup-stale-sessions >/dev/null 2>&1\"",
        ]
        .map(str::to_owned)
        .to_vec();

        assert_eq!(
            calls,
            vec![(
                TmuxTarget::Recorded(Some("/fixture/tmux.sock".to_string())),
                expected_args,
            )]
        );
    }

    #[test]
    fn cleanup_command_quotes_paths_for_the_shell_and_tmux() {
        assert_eq!(
            cleanup_command(
                "/fixture/bin/a path",
                &[
                    ("HOME", "/fixture/home with space"),
                    ("XDG_CACHE_HOME", "/fixture/cache/#$data"),
                ],
            ),
            "run-shell -b \"HOME='/fixture/home with space' XDG_CACHE_HOME='/fixture/cache/##\\$data' '/fixture/bin/a path' agent cleanup-stale-sessions >/dev/null 2>&1\"",
        );
    }
}
