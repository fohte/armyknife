use anyhow::Result;
use clap::Args;

use crate::commands::agent::peer::notify;
#[cfg(not(test))]
use crate::infra::process;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct UnreportedStopNotificationArgs {
    #[arg(long)]
    pub session_id: String,
    #[arg(long)]
    pub recipient_session_id: String,
    #[arg(long)]
    pub message: String,
}

#[cfg(not(test))]
pub(super) fn spawn(session_id: &str, recipient_session_id: &str, message: &str) {
    process::spawn_self_detached(
        "agent.hook.unreported_stop.spawn",
        "agent.hook.unreported_stop.spawn_failed",
        session_id,
        &[
            "agent",
            "unreported-stop-notification",
            "--session-id",
            session_id,
            "--recipient-session-id",
            recipient_session_id,
            "--message",
            message,
        ],
    );
}

pub fn run(args: &UnreportedStopNotificationArgs) -> Result<()> {
    if let Err(error) = notify::notify(&args.recipient_session_id, &args.message, None, None) {
        tracing::warn!(
            event = "agent.hook.unreported_stop.delivery_failed",
            session = %args.session_id,
            recipient_session = %args.recipient_session_id,
            error = %error,
        );
    }
    Ok(())
}
