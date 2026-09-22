//! Launches Codex while recording its thread ID in the invoking tmux pane.

use std::ffi::OsString;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Args;

use super::codex_steer;
use super::types::TMUX_SESSION_OPTION;
use crate::infra::tmux;
use crate::shared::command::{self, find_command_path};

const THREAD_STARTED_TIMEOUT: Duration = Duration::from_secs(60 * 60);

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

    let Some(pane_id) = std::env::var("TMUX_PANE")
        .ok()
        .filter(|pane_id| !pane_id.is_empty())
    else {
        return finish_child_status(command.status().context("Failed to start codex")?);
    };

    let cwd = std::env::current_dir().context("Failed to determine current directory")?;
    let launch_lock = codex_steer::acquire_launch_lock(&cwd)?;
    let mut client = codex_steer::Client::connect()?;

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
                        eprintln!(
                            "[armyknife] warning: failed to record Codex thread ID in pane {pane_id}: {error}"
                        );
                    }
                }
                Err(error) => {
                    eprintln!("[armyknife] warning: failed to capture Codex thread ID: {error:#}");
                }
            }
            drop(launch_lock);
        })
        .context("Failed to start Codex thread-binding watcher")?;

    let mut child = command.spawn().context("Failed to start codex")?;
    finish_child_status(child.wait().context("Failed to wait for codex")?)
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
