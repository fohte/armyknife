//! Deferred self-archive for worktree cleanup.
//!
//! A Codex tool can run `a wm delete` inside the thread that cleanup must
//! archive. This detached worker waits for cleanup to exit before archiving it.

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Args;

use crate::commands::agent::codex_steer;
use crate::infra::process;

const PARENT_EXIT_TIMEOUT: Duration = Duration::from_secs(300);
const PARENT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const EVENT_TARGET: &str = "armyknife::commands::agent::archive_detached";

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ArchiveDetachedArgs {
    /// Codex thread to archive after the cleanup process exits.
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
        "archive-detached",
        "--thread-id",
        thread_id,
        "--parent-pid",
        parent_pid.as_str(),
    ];
    let cwd = crate::shared::dirs::home_dir();

    process::spawn_detached(exe, args, cwd.as_deref(), &[])
        .context("failed to start deferred Codex thread archive")
}

pub fn run(args: &ArchiveDetachedArgs) -> Result<()> {
    if !wait_for_parent_exit(args.parent_pid) {
        tracing::warn!(
            target: EVENT_TARGET,
            event = "agent.codex_archive.parent_timeout",
            thread_id = %args.thread_id,
            parent_pid = args.parent_pid,
        );
        return Ok(());
    }

    if let Err(error) = codex_steer::archive_thread(&args.thread_id) {
        tracing::warn!(
            target: EVENT_TARGET,
            event = "agent.codex_archive.err",
            thread_id = %args.thread_id,
            msg = format!("failed to archive Codex thread: {error:#}"),
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
