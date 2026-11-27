use std::env;

use anyhow::Context;

use crate::commands::agent::types::HookEvent;
use crate::infra::tmux;

const HOOK_INDEX: usize = 1000;
const TMUX_HOOKS: [&str; 4] = [
    "pane-exited",
    "pane-died",
    "after-kill-pane",
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

pub(super) fn should_configure(event: HookEvent, inside_tmux: bool, pane_resolved: bool) -> bool {
    event == HookEvent::SessionStart && (inside_tmux || pane_resolved)
}

pub(super) fn configure_current() -> anyhow::Result<()> {
    configure_from_current_process(tmux::run_tmux)
}

pub(super) fn configure_for_socket(socket_path: Option<&str>) -> anyhow::Result<()> {
    configure_from_current_process(|args| tmux::run_tmux_for_socket(socket_path, args))
}

fn configure_from_current_process(
    run_tmux: impl FnMut(&[&str]) -> tmux::Result<()>,
) -> anyhow::Result<()> {
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

    configure(executable, &environment, run_tmux)?;
    Ok(())
}

fn configure(
    executable: &str,
    environment: &[(&str, &str)],
    mut run_tmux: impl FnMut(&[&str]) -> tmux::Result<()>,
) -> tmux::Result<()> {
    let command = cleanup_command(executable, environment);
    for hook in TMUX_HOOKS {
        let indexed_hook = format!("{hook}[{HOOK_INDEX}]");
        run_tmux(&["set-hook", "-g", &indexed_hook, &command])?;
    }

    Ok(())
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

    format!("run-shell -b {}", tmux_double_quote(&shell_command))
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
    use super::{configure, should_configure};
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
    fn configure_sets_each_global_cleanup_hook_at_the_fixed_index() {
        let mut calls = Vec::new();

        configure(
            "/fixture/bin/a",
            &[
                ("PATH", "/fixture/bin:/usr/bin"),
                ("XDG_CACHE_HOME", "/fixture/cache"),
            ],
            |args| {
                calls.push(args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
                Ok(())
            },
        )
        .expect("tmux hook setup should succeed");

        assert_eq!(
            calls,
            vec![
                vec![
                    "set-hook",
                    "-g",
                    "pane-exited[1000]",
                    "run-shell -b \"PATH=/fixture/bin:/usr/bin XDG_CACHE_HOME=/fixture/cache /fixture/bin/a agent cleanup-stale-sessions\"",
                ],
                vec![
                    "set-hook",
                    "-g",
                    "pane-died[1000]",
                    "run-shell -b \"PATH=/fixture/bin:/usr/bin XDG_CACHE_HOME=/fixture/cache /fixture/bin/a agent cleanup-stale-sessions\"",
                ],
                vec![
                    "set-hook",
                    "-g",
                    "after-kill-pane[1000]",
                    "run-shell -b \"PATH=/fixture/bin:/usr/bin XDG_CACHE_HOME=/fixture/cache /fixture/bin/a agent cleanup-stale-sessions\"",
                ],
                vec![
                    "set-hook",
                    "-g",
                    "session-closed[1000]",
                    "run-shell -b \"PATH=/fixture/bin:/usr/bin XDG_CACHE_HOME=/fixture/cache /fixture/bin/a agent cleanup-stale-sessions\"",
                ],
            ]
        );
    }

    #[test]
    fn cleanup_command_quotes_paths_for_the_shell_and_tmux() {
        let mut calls = Vec::new();

        configure(
            "/fixture/bin/a path",
            &[
                ("HOME", "/fixture/home with space"),
                ("XDG_CACHE_HOME", "/fixture/cache/#$data"),
            ],
            |args| {
                calls.push(args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
                Ok(())
            },
        )
        .expect("tmux hook setup should succeed");

        assert_eq!(
            calls[0],
            vec![
                "set-hook",
                "-g",
                "pane-exited[1000]",
                "run-shell -b \"HOME='/fixture/home with space' XDG_CACHE_HOME='/fixture/cache/##\\$data' '/fixture/bin/a path' agent cleanup-stale-sessions\"",
            ],
        );
    }
}
