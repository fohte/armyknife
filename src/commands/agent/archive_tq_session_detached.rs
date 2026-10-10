//! `a agent archive-tq-session-detached` (hidden) subcommand.
//!
//! Spawned once a session is confirmed `Ended` (never `Paused` — a paused
//! session must stay resumable), after a confirmed session end or when a
//! different session takes over its tmux pane. tq can hang or answer slowly,
//! so lifecycle handlers never wait on it directly: archiving happens in this
//! separate detached process instead. Best-effort: failures are logged and
//! ignored so they do not block session shutdown.

use anyhow::Result;
use clap::Args;

#[cfg(not(test))]
use crate::infra::process;
use crate::infra::tq::TqClient;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ArchiveTqSessionDetachedArgs {
    /// Claude Code session_id to archive in tq.
    #[arg(long)]
    pub session: String,
}

/// Spawns a detached `a agent archive-tq-session-detached --session <id>` so
/// the hook can return immediately. Errors are logged, not surfaced —
/// failing the hook over an opportunistic archive is the wrong trade.
#[cfg(not(test))]
pub fn spawn_in_background(session_id: &str) {
    process::spawn_self_detached(
        "agent.tq_archive.spawn",
        "agent.tq_archive.spawn_failed",
        session_id,
        &[
            "agent",
            "archive-tq-session-detached",
            "--session",
            session_id,
        ],
    );
}

pub fn run(args: &ArchiveTqSessionDetachedArgs) -> Result<()> {
    let Some(client) = TqClient::detect() else {
        return Ok(());
    };
    if let Err(e) = client.archive_session(&args.session) {
        tracing::warn!(
            event = "agent.tq_archive.failed",
            session = %args.session,
            error = %e,
        );
    }
    Ok(())
}
