//! `a cc delete-tq-session-detached` (hidden) subcommand.
//!
//! Spawned once a session is confirmed `Ended` (never `Paused` — a paused
//! session must stay resumable), either by a genuine `SessionEnd`, or by
//! `evict_paused_sessions_on_pane_takeover` (see `cc::hook`) evicting a
//! stale `Paused` session whose tmux pane was taken over by a different
//! session. tq lives behind Cloudflare Access and can hang or answer
//! slowly, so the hook never waits on it directly: deletion happens in this
//! separate detached process instead. Best-effort — tq performs its own
//! periodic cleanup of stale sessions regardless, so a failed or skipped
//! deletion here is never the only cleanup path.

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
    process::spawn_self_detached(
        "cc.tq_delete.spawn",
        "cc.tq_delete.spawn_failed",
        session_id,
        &["cc", "delete-tq-session-detached", "--session", session_id],
    );
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
