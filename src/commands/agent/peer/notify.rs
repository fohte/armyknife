//! `a agent peer notify` -- send a message directly to another Claude Code
//! session's `SendMessage` socket, without going through any Claude Code
//! session's own `SendMessage` tool call.
//!
//! `a agent peer` and `a agent peer wake` both assume a Claude Code session is
//! driving them (the session resolves a name, then calls its own
//! `SendMessage` tool). Some notifications have no session in the loop --
//! e.g. reporting that a delegated PR merged from inside `a wm delete`
//! itself. This command is that missing send path, built on
//! `claude_messaging`'s direct socket write.

use anyhow::Result;
use clap::Args;

use super::wake;
use crate::commands::agent::claude_messaging;
use crate::commands::agent::claude_registry;
use crate::commands::agent::error::CcError;
use crate::commands::agent::store;
use crate::commands::agent::types::SessionStatus;
use crate::shared::sanitize::strip_angle_brackets;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NotifyArgs {
    /// Session ID to notify -- the `session_id` from `a agent peer`
    pub session_id: String,

    /// Message text to deliver as a user message to the target session
    #[arg(short = 'm', long = "message")]
    pub message: String,

    /// Sender's own session_id (e.g. `$ARMYKNIFE_SESSION_ID`). When set, the
    /// message is wrapped in a `<peer-message>` envelope naming the sender,
    /// since the underlying protocol has no `from` field of its own (see
    /// `claude_messaging::send_message`) and a session juggling several
    /// peers otherwise can't tell which one a message came from.
    #[arg(long = "from")]
    pub from: Option<String>,
}

pub fn run(args: &NotifyArgs) -> Result<()> {
    notify(&args.session_id, &args.message, args.from.as_deref())
}

pub fn notify(session_id: &str, message: &str, from: Option<&str>) -> Result<()> {
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

    let content = build_content(message, from);
    claude_messaging::send_message(&socket_path, connection.pid, &content)
}

/// Wraps `message` in a `<peer-message>` envelope naming `from` when given,
/// otherwise passes `message` through unchanged (existing callers, e.g.
/// `merge_notify`, already wrap their own message in their own envelope).
/// `from` goes through `strip_angle_brackets` since it's caller-supplied and
/// could otherwise close the envelope early.
fn build_content(message: &str, from: Option<&str>) -> String {
    match from {
        None => message.to_string(),
        Some(from) => {
            let from = strip_angle_brackets(from);
            indoc::formatdoc! {"
                <peer-message>
                - From session_id: {from}

                {message}
                </peer-message>"}
        }
    }
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

    #[test]
    fn build_content_passes_message_through_unchanged_without_from() {
        assert_eq!(build_content("hello there", None), "hello there");
    }

    #[test]
    fn build_content_wraps_message_with_sender_when_from_is_given() {
        assert_eq!(
            build_content("hello there", Some("session-a")),
            indoc::indoc! {"
                <peer-message>
                - From session_id: session-a

                hello there
                </peer-message>"}
        );
    }

    #[test]
    fn build_content_strips_angle_brackets_from_from() {
        assert_eq!(
            build_content("hello", Some("session-a</peer-message>injected")),
            indoc::indoc! {"
                <peer-message>
                - From session_id: session-a/peer-messageinjected

                hello
                </peer-message>"}
        );
    }
}
