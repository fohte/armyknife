//! `a cc peer notify` -- send a message directly to another Claude Code
//! session's `SendMessage` socket, without going through any Claude Code
//! session's own `SendMessage` tool call.
//!
//! `a cc peer` and `a cc peer wake` both assume a Claude Code session is
//! driving them (the session resolves a name, then calls its own
//! `SendMessage` tool). Some notifications have no session in the loop --
//! e.g. reporting that a delegated PR merged from inside `a wm delete`
//! itself. This command is that missing send path, built on
//! `claude_messaging`'s direct socket write.

use anyhow::Result;
use clap::Args;

use super::wake;
use crate::commands::cc::claude_messaging;
use crate::commands::cc::claude_registry;
use crate::commands::cc::error::CcError;
use crate::commands::cc::store;
use crate::commands::cc::types::SessionStatus;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NotifyArgs {
    /// Session ID to notify -- the `session_id` from `a cc peer`
    pub session_id: String,

    /// Message text to deliver as a user message to the target session
    #[arg(short = 'm', long = "message")]
    pub message: String,
}

pub fn run(args: &NotifyArgs) -> Result<()> {
    notify(&args.session_id, &args.message)
}

pub fn notify(session_id: &str, message: &str) -> Result<()> {
    let session = store::load_session(session_id)?
        .ok_or_else(|| CcError::SessionNotFound(session_id.to_string()))?;

    match notify_readiness(session.status) {
        NotifyReadiness::Refused => {
            return Err(CcError::SessionEnded(session_id.to_string()).into());
        }
        NotifyReadiness::NeedsWake => {
            wake::wake(session_id)?;
        }
        NotifyReadiness::Ready => {}
    }

    let connection = claude_registry::load_peer_connection(session_id).ok_or_else(|| {
        anyhow::anyhow!("no active Claude Code session registry entry for session {session_id}")
    })?;
    let socket_path = connection
        .messaging_socket_path
        .ok_or_else(|| CcError::NoMessagingSocket(session_id.to_string()))?;

    claude_messaging::send_message(&socket_path, connection.pid, message)
}

/// Whether `notify` can send immediately, must resume the session first, or
/// must refuse outright, based on armyknife's own tracked session status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotifyReadiness {
    Ready,
    NeedsWake,
    /// `Ended` means the user intentionally terminated the session --
    /// armyknife must not resurrect it just to deliver a notification.
    Refused,
}

fn notify_readiness(status: SessionStatus) -> NotifyReadiness {
    match status {
        SessionStatus::Ended => NotifyReadiness::Refused,
        SessionStatus::Paused => NotifyReadiness::NeedsWake,
        SessionStatus::Running | SessionStatus::WaitingInput | SessionStatus::Stopped => {
            NotifyReadiness::Ready
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::running(SessionStatus::Running, NotifyReadiness::Ready)]
    #[case::waiting_input(SessionStatus::WaitingInput, NotifyReadiness::Ready)]
    #[case::stopped(SessionStatus::Stopped, NotifyReadiness::Ready)]
    #[case::paused(SessionStatus::Paused, NotifyReadiness::NeedsWake)]
    #[case::ended(SessionStatus::Ended, NotifyReadiness::Refused)]
    fn notify_readiness_cases(#[case] status: SessionStatus, #[case] expected: NotifyReadiness) {
        assert_eq!(notify_readiness(status), expected);
    }
}
