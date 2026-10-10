//! Launches Codex while recording its thread ID in the invoking tmux pane.

use std::ffi::OsString;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Args;

use super::codex_steer;
use super::types::{TMUX_SESSION_OPTION, resolve_session_option};
use crate::infra::tmux;
use crate::shared::command::{self, find_command_path};
use crate::shared::env_var::EnvVars;

const THREAD_STARTED_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const LAUNCH_LOCK_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Args, Clone, PartialEq, Eq)]
#[command(disable_help_flag = true)]
pub struct CodexArgs {
    /// Arguments forwarded to the Codex CLI.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<OsString>,
}

pub fn run(args: &CodexArgs) -> Result<()> {
    let binary_path =
        find_command_path("codex").context("Could not find 'codex' command in PATH")?;
    let mut command = command::new(&binary_path);
    command.args(&args.args);

    let pane_id = std::env::var("TMUX_PANE")
        .ok()
        .filter(|pane_id| !pane_id.is_empty());
    let managed_launch = std::env::var_os(EnvVars::codex_managed_launch_name()).is_some();
    let session_pane_id = pane_id.clone();
    let Some(pane_id) = pane_id_for_binding(pane_id, managed_launch) else {
        let status = command.status().context("Failed to start codex")?;
        log_session_end_error(end_bound_codex_session(session_pane_id.as_deref()));
        return finish_child_status(status);
    };

    let cwd = std::env::current_dir().context("Failed to determine current directory")?;
    let launch_lock = codex_steer::acquire_launch_lock_with_timeout(&cwd, LAUNCH_LOCK_TIMEOUT)
        .context("Another Codex launch in this directory is still starting; retry shortly")?;
    let mut client = match codex_steer::Client::connect() {
        Ok(client) => client,
        Err(error) => {
            drop(launch_lock);
            tracing::warn!(
                event = "agent.codex.thread_binding.err",
                error = %error,
                "Codex app-server is unavailable; starting Codex without pane binding"
            );
            let status = command.status().context("Failed to start codex")?;
            log_session_end_error(end_bound_codex_session(session_pane_id.as_deref()));
            return finish_child_status(status);
        }
    };
    let (child_finished_tx, child_finished_rx) = mpsc::channel();

    let _watcher = thread::Builder::new()
        .name("codex-thread-binding".to_string())
        .spawn(move || {
            let thread_id =
                client.wait_for_thread_started_with_timeout(&cwd, THREAD_STARTED_TIMEOUT);
            match thread_id {
                Ok(thread_id) => {
                    if let Err(error) =
                        tmux::set_pane_option(&pane_id, TMUX_SESSION_OPTION, &thread_id)
                    {
                        tracing::warn!(
                            event = "agent.codex.thread_binding.err",
                            pane_id,
                            thread_id,
                            error = %error,
                            "failed to record Codex thread ID in tmux pane"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        event = "agent.codex.thread_binding.err",
                        error = %error,
                        "failed to capture Codex thread ID"
                    );
                    let _ = child_finished_rx.recv();
                }
            }
            drop(launch_lock);
        })
        .context("Failed to start Codex thread-binding watcher")?;

    let mut child = command.spawn().context("Failed to start codex")?;
    let status = child.wait().context("Failed to wait for codex")?;
    let _ = child_finished_tx.send(());
    log_session_end_error(end_bound_codex_session(session_pane_id.as_deref()));
    finish_child_status(status)
}

fn end_bound_codex_session(pane_id: Option<&str>) -> Result<()> {
    end_bound_codex_session_with(
        pane_id,
        |pane_id| resolve_session_option(|option| tmux::get_pane_option(pane_id, option)),
        super::hook::codex_process_exited,
    )
}

fn end_bound_codex_session_with(
    pane_id: Option<&str>,
    resolve_session_id: impl FnOnce(&str) -> Option<String>,
    end_session: impl FnOnce(&str) -> Result<()>,
) -> Result<()> {
    let Some(pane_id) = pane_id else {
        return Ok(());
    };
    if let Some(session_id) = resolve_session_id(pane_id) {
        end_session(&session_id)?;
    }
    Ok(())
}

fn log_session_end_error(result: Result<()>) {
    if let Err(error) = result {
        tracing::warn!(
            event = "agent.codex.session_end.err",
            error = %error,
            "failed to mark Codex session ended after CLI exit"
        );
    }
}

fn pane_id_for_binding(pane_id: Option<String>, managed_launch: bool) -> Option<String> {
    if managed_launch { None } else { pane_id }
}

fn finish_child_status(status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        std::process::exit(
            status
                .code()
                .unwrap_or_else(|| status.signal().map_or(1, |signal| 128 + signal)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{end_bound_codex_session_with, pane_id_for_binding};
    use rstest::rstest;

    #[rstest]
    #[case::managed_launch_skips_binding(Some("%pane-a"), true, None)]
    #[case::manual_tmux_launch_binds(Some("%pane-a"), false, Some("%pane-a"))]
    #[case::outside_tmux_skips_binding(None, false, None)]
    fn pane_binding_cases(
        #[case] pane_id: Option<&str>,
        #[case] managed_launch: bool,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(
            pane_id_for_binding(pane_id.map(str::to_owned), managed_launch),
            expected.map(str::to_owned),
        );
    }

    #[test]
    fn child_exit_ends_the_session_bound_to_its_pane() {
        let mut ended = Vec::new();

        let result = end_bound_codex_session_with(
            Some("%pane-a"),
            |_| Some("thread-one".to_string()),
            |session_id| {
                ended.push(session_id.to_string());
                Ok(())
            },
        );

        assert_eq!(
            (result.is_ok(), ended),
            (true, vec!["thread-one".to_string()])
        );
    }
}
