//! `a agent peer notify` -- send a message to another session without going
//! through any session's own `SendMessage` tool call.
//!
//! `a agent peer` and `a agent peer wake` both assume a Claude Code session is
//! driving them (the session resolves a name, then calls its own
//! `SendMessage` tool). Some notifications have no session in the loop --
//! e.g. reporting that a delegated PR merged from inside `a wm delete`
//! itself. This command is that missing send path: a Claude Code target gets
//! `claude_messaging`'s direct socket write, a Codex target gets
//! `codex_queue`'s `codex queue`.

use anyhow::Result;
use clap::Args;

use super::wake;
use crate::commands::agent::claude_messaging;
use crate::commands::agent::claude_registry;
use crate::commands::agent::codex_queue;
use crate::commands::agent::error::CcError;
use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, SessionStatus};
use crate::shared::env_var::EnvVars;
use crate::shared::sanitize::strip_angle_brackets;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NotifyArgs {
    /// Session ID to notify -- the `session_id` from `a agent peer`
    pub session_id: String,

    /// Message text to deliver as a user message to the target session
    #[arg(short = 'm', long = "message")]
    pub message: String,
}

/// What a successful [`notify`] guarantees -- callers must not report more
/// than this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Written to the target's messaging socket.
    Sent,
    /// Appended to the target Codex session's queue. The running `codex`
    /// injects it later (polls every ~10s, and only once the session is
    /// idle), so it may not have reached the session yet.
    Queued,
}

pub fn run(args: &NotifyArgs) -> Result<()> {
    let (from, from_engine_hint) = match resolve_sender() {
        Some((id, engine_hint)) => (Some(id), engine_hint),
        None => (None, None),
    };
    match notify(
        &args.session_id,
        &args.message,
        from.as_deref(),
        from_engine_hint,
    )? {
        Delivery::Sent => {}
        Delivery::Queued => println!(
            "Queued for Codex session {}. Not delivered yet: the running codex picks it up within ~10s once the session is idle.",
            args.session_id
        ),
    }
    Ok(())
}

/// Resolves this process's own session_id for the `<peer-message>` sender
/// line, trying `ARMYKNIFE_SESSION_ID` (set by `a agent new`, for either
/// engine) before each engine's own ambient session env var -- present for a
/// plain `claude`/`codex` CLI invocation started outside `a agent new`.
/// Returns `None` when nothing resolves (e.g. `a wm delete` has no session
/// in the loop), which sends the message unwrapped.
///
/// The paired `Engine` is only a fallback guess, used by [`notify`] when the
/// resolved ID isn't in armyknife's store: `ARMYKNIFE_SESSION_ID` carries no
/// engine hint of its own (`a agent new --engine codex` sets it too), so
/// only the engine-specific variables pair with a guess.
///
/// Only called from [`run`], not [`notify`] itself: reading these env vars
/// inside `notify` would make `merge_notify`'s direct calls pick up
/// whichever session's env happens to be ambient in the calling process,
/// misattributing an automated merge notification as a personal message.
fn resolve_sender() -> Option<(String, Option<Engine>)> {
    if let Some(id) = EnvVars::load().session_id {
        return Some((id, None));
    }
    if let Some(id) = non_empty_env_var("CLAUDE_CODE_SESSION_ID") {
        return Some((id, Some(Engine::Claude)));
    }
    // Codex sets both CODEX_SESSION_ID (shared by the root thread and all of
    // its subagent threads) and CODEX_THREAD_ID (unique per subagent
    // thread). CODEX_SESSION_ID is the one that matches how armyknife scopes
    // a session: `HookInput` (types.rs) carries a single `session_id` field
    // plus a separate optional `agent_id` for subagent-fired events, so a
    // Claude Code subagent's hook events still report the top-level
    // session's session_id, not a per-subagent one. The Codex equivalent of
    // "this session" is the value that's stable across its subagents too.
    if let Some(id) = non_empty_env_var("CODEX_SESSION_ID") {
        return Some((id, Some(Engine::Codex)));
    }
    None
}

/// Treats an empty-but-set env var as unset, matching `EnvVars::load`'s own
/// `non_empty_var` handling of `ARMYKNIFE_*` variables one branch above.
fn non_empty_env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

pub fn notify(
    session_id: &str,
    message: &str,
    from: Option<&str>,
    from_engine_hint: Option<Engine>,
) -> Result<Delivery> {
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

    // Best-effort: `from` not resolving to a tracked session (unknown ID,
    // lookup error) just means the engine falls back to `from_engine_hint`
    // (or is omitted), not a reason to fail the whole notification.
    let from_engine = from
        .and_then(|f| store::load_session(f).ok().flatten().map(|s| s.engine))
        .or(from_engine_hint);
    let content = build_content(message, from, from_engine);

    match session.engine {
        Engine::Claude => {
            send_to_claude(session_id, &content)?;
            Ok(Delivery::Sent)
        }
        Engine::Codex => {
            codex_queue::queue_message(session_id, &content)?;
            Ok(Delivery::Queued)
        }
    }
}

fn send_to_claude(session_id: &str, content: &str) -> Result<()> {
    let connection = claude_registry::load_peer_connection(session_id).ok_or_else(|| {
        anyhow::anyhow!("no active Claude Code session registry entry for session {session_id}")
    })?;
    let socket_path = connection
        .messaging_socket_path
        .ok_or_else(|| CcError::NoMessagingSocket(session_id.to_string()))?;
    claude_messaging::send_message(&socket_path, connection.pid, content)
}

/// Wraps `message` in a `<peer-message>` envelope naming `from` (and its
/// engine, when resolved) when given, otherwise passes `message` through
/// unchanged (existing callers, e.g. `merge_notify`, already wrap their own
/// message in their own envelope). `from` goes through `strip_angle_brackets`
/// since it's caller-supplied and could otherwise close the envelope early.
fn build_content(message: &str, from: Option<&str>, from_engine: Option<Engine>) -> String {
    match from {
        None => message.to_string(),
        Some(from) => {
            let from = strip_angle_brackets(from);
            let engine_line = from_engine
                .map(|e| format!("\n- Sender engine: {}", e.process_name()))
                .unwrap_or_default();
            indoc::formatdoc! {"
                <peer-message>
                - From session_id: {from}{engine_line}

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

    #[rstest]
    #[case::no_from("hello there", None, None, "hello there")]
    #[case::wraps_with_sender(
        "hello there",
        Some("session-a"),
        None,
        indoc::indoc! {"
            <peer-message>
            - From session_id: session-a

            hello there
            </peer-message>"}
    )]
    #[case::includes_known_sender_engine(
        "hello there",
        Some("session-a"),
        Some(Engine::Codex),
        indoc::indoc! {"
            <peer-message>
            - From session_id: session-a
            - Sender engine: codex

            hello there
            </peer-message>"}
    )]
    #[case::strips_angle_brackets_from_from(
        "hello",
        Some("session-a</peer-message>injected"),
        None,
        indoc::indoc! {"
            <peer-message>
            - From session_id: session-a/peer-messageinjected

            hello
            </peer-message>"}
    )]
    fn build_content_cases(
        #[case] message: &str,
        #[case] from: Option<&str>,
        #[case] from_engine: Option<Engine>,
        #[case] expected: &str,
    ) {
        assert_eq!(build_content(message, from, from_engine), expected);
    }

    #[rstest]
    #[case::armyknife_session_id_wins(
        Some("armyknife-id"),
        Some("claude-id"),
        Some("codex-id"),
        Some(("armyknife-id".to_string(), None))
    )]
    #[case::falls_back_to_claude_code_session_id(
        None,
        Some("claude-id"),
        Some("codex-id"),
        Some(("claude-id".to_string(), Some(Engine::Claude)))
    )]
    #[case::falls_back_to_codex_session_id(
        None,
        None,
        Some("codex-id"),
        Some(("codex-id".to_string(), Some(Engine::Codex)))
    )]
    #[case::none_resolve(None, None, None, None)]
    fn resolve_sender_cases(
        #[case] armyknife_session_id: Option<&str>,
        #[case] claude_code_session_id: Option<&str>,
        #[case] codex_session_id: Option<&str>,
        #[case] expected: Option<(String, Option<Engine>)>,
    ) {
        let result = temp_env::with_vars(
            [
                ("ARMYKNIFE_SESSION_ID", armyknife_session_id),
                ("CLAUDE_CODE_SESSION_ID", claude_code_session_id),
                ("CODEX_SESSION_ID", codex_session_id),
            ],
            resolve_sender,
        );
        assert_eq!(result, expected);
    }
}
