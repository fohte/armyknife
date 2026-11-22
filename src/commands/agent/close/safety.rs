use std::io::{self, Write};
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::Utc;

use super::super::types::{Session, SessionStatus, TmuxInfo};
use super::{CloseRuntime, DraftInspection};

pub(super) fn ensure_session_is_idle(
    session: &Session,
    effective_status: SessionStatus,
) -> Result<()> {
    if session.has_active_work(effective_status) {
        match effective_status {
            SessionStatus::Running | SessionStatus::WaitingInput => bail!(
                "Agent session `{}` is running or waiting for input; complete any requested input and wait for the current turn to finish, then retry this command",
                session.session_id
            ),
            _ => bail!(
                "Agent session `{}` has pending background or agent tasks; wait for them to finish, then retry this command",
                session.session_id
            ),
        }
    }
    if session.has_pending_permission_requests() {
        bail!(
            "Agent session `{}` has pending permission requests; resolve them and retry this command",
            session.session_id
        );
    }
    Ok(())
}

pub(super) fn check_draft_state<R: CloseRuntime>(
    runtime: &R,
    tmux_info: &TmuxInfo,
    session: &Session,
) -> Result<DraftInspection> {
    match runtime.has_draft(tmux_info, session.engine) {
        Some(false) => Ok(DraftInspection::NoDraft),
        Some(true) => bail!(
            "Agent session `{}` has an unsent draft; pass --force to close it",
            session.session_id
        ),
        None => check_unknown_draft_state(session, runtime.unknown_draft_timeout()),
    }
}

pub(super) fn check_unknown_draft_state(
    session: &Session,
    timeout: Duration,
) -> Result<DraftInspection> {
    let active = match Utc::now()
        .signed_duration_since(session.updated_at)
        .to_std()
    {
        Ok(elapsed) => elapsed < timeout,
        Err(_) => true,
    };
    tracing::debug!(
        event = "agent.close.draft_state_unavailable",
        session = %session.session_id,
        timeout = ?timeout,
        decision = if active { "blocked_recent_activity" } else { "proceed_after_timeout" },
    );
    if active {
        bail!(
            "Could not check for an unsent draft in agent session `{}`; recent activity prevents closing it, pass --force to close it",
            session.session_id
        );
    }
    Ok(DraftInspection::Unavailable)
}

pub(super) fn validate_session_without_tmux_info(
    session: &Session,
    unknown_draft_timeout: Duration,
) -> Result<DraftInspection> {
    ensure_session_is_idle(session, session.status)?;
    check_unknown_draft_state(session, unknown_draft_timeout)
}

pub(super) fn write_draft_unavailable_warning(
    writer: &mut impl Write,
    session_id: &str,
) -> io::Result<()> {
    writeln!(
        writer,
        "Warning: could not inspect the unsent draft for session `{session_id}`; using its last update time for the close safety check"
    )
}
