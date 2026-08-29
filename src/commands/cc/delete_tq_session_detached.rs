//! `a cc delete-tq-session-detached` (hidden) subcommand.
//!
//! Spawned by the SessionEnd hook once a session is confirmed `Ended` (never
//! `Paused` — a paused session must stay resumable). tq lives behind
//! Cloudflare Access and can hang or answer slowly, so the hook never waits
//! on it directly: deletion happens in this separate detached process
//! instead. Best-effort — tq's own 30-day retention is the fallback if this
//! never runs or fails.

use anyhow::Result;
use clap::Args;

use crate::infra::process;
use crate::infra::tq::TqClient;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct DeleteTqSessionDetachedArgs {
    /// Claude Code session_id to delete from tq.
    #[arg(long)]
    pub session: String,
}

/// Spawns a detached `a cc delete-tq-session-detached --session <id>` so the
/// hook can return immediately. Errors are logged, not surfaced — failing
/// the hook over an opportunistic cleanup is the wrong trade.
pub fn spawn_in_background(session_id: &str) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                event = "cc.tq_delete.spawn_failed",
                session = session_id,
                reason = "current_exe",
                error = %e,
            );
            return;
        }
    };
    let result = process::spawn_detached(
        exe,
        ["cc", "delete-tq-session-detached", "--session", session_id],
        None,
        &[],
    );
    if let Err(e) = result {
        tracing::warn!(
            event = "cc.tq_delete.spawn_failed",
            session = session_id,
            reason = "spawn_detached",
            error = %e,
        );
    }
}

pub fn run(args: &DeleteTqSessionDetachedArgs) -> Result<()> {
    let Some(client) = TqClient::detect() else {
        return Ok(());
    };
    if let Err(e) = client.delete_session(&args.session) {
        tracing::warn!(
            event = "cc.tq_delete.failed",
            session = %args.session,
            error = %e,
        );
    }
    Ok(())
}
