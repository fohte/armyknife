//! Deferred self-interruption for worktree cleanup.
//!
//! A Codex tool can run `a wm delete` inside the turn that cleanup must stop.
//! This detached worker waits for the cleanup process to exit before sending
//! the interrupt, so the tool call can finish deleting sessions and windows.

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Args;

use crate::commands::agent::codex_steer;
use crate::infra::process;

const PARENT_EXIT_TIMEOUT: Duration = Duration::from_secs(300);
const PARENT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const EVENT_TARGET: &str = "armyknife::commands::agent::interrupt_detached";

#[derive(Args, Clone, PartialEq, Eq)]
pub struct InterruptDetachedArgs {
    /// Codex thread to interrupt after the cleanup process exits.
    #[arg(long)]
    pub thread_id: String,
    /// Cleanup process whose exit indicates all worktree resources are handled.
    #[arg(long)]
    pub parent_pid: u32,
}

pub fn spawn_after_parent_exit(thread_id: &str) -> Result<()> {
    let exe = std::env::current_exe().context("failed to resolve armyknife executable")?;
    let parent_pid = std::process::id().to_string();
    let args = [
        "agent",
        "interrupt-detached",
        "--thread-id",
        thread_id,
        "--parent-pid",
        parent_pid.as_str(),
    ];
    let cwd = crate::shared::dirs::home_dir();

    process::spawn_detached(exe, args, cwd.as_deref(), &[])
        .context("failed to start deferred Codex turn interruption")
}

pub fn run(args: &InterruptDetachedArgs) -> Result<()> {
    let mut client = match codex_steer::connect_for_interrupt() {
        Ok(Some(client)) => client,
        Ok(None) => return Ok(()),
        Err(error) => {
            tracing::warn!(
                target: EVENT_TARGET,
                event = "agent.codex_interrupt.connect_err",
                thread_id = %args.thread_id,
                msg = format!("failed to connect to Codex app-server: {error:#}"),
            );
            return Ok(());
        }
    };

    if !wait_for_parent_exit(args.parent_pid) {
        tracing::warn!(
            target: EVENT_TARGET,
            event = "agent.codex_interrupt.parent_timeout",
            thread_id = %args.thread_id,
            parent_pid = args.parent_pid,
        );
        return Ok(());
    }

    if let Err(error) = client.interrupt_if_in_progress(&args.thread_id) {
        tracing::warn!(
            target: EVENT_TARGET,
            event = "agent.codex_interrupt.err",
            thread_id = %args.thread_id,
            msg = format!("failed to interrupt Codex turn: {error:#}"),
        );
    }

    Ok(())
}

fn wait_for_parent_exit(parent_pid: u32) -> bool {
    let deadline = Instant::now() + PARENT_EXIT_TIMEOUT;
    while process_is_alive(parent_pid) {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(PARENT_POLL_INTERVAL);
    }
    true
}

fn process_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: signal 0 only checks whether this process ID exists.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}
