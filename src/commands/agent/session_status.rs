use anyhow::Result;

use super::bg_tasks;
use super::store;
use super::types::{BG_RUN_PENDING_TASK_MARKER, Session};

/// Loads one session with `a agent bg run` registry state included for
/// presentation and background-task decisions.
pub(crate) fn load_session_with_bg_run_status(session_id: &str) -> Result<Option<Session>> {
    let Some(mut session) = store::load_session(session_id)? else {
        return Ok(None);
    };
    include_pending_status(&mut session);
    Ok(Some(session))
}

/// Lists active sessions with `a agent bg run` registry state included for
/// presentation and background-task decisions.
pub(crate) fn list_sessions_with_bg_run_status() -> Result<Vec<Session>> {
    let mut sessions = store::list_sessions()?;
    for session in &mut sessions {
        include_pending_status(session);
    }
    Ok(sessions)
}

/// Includes registry state in the in-memory task set consumed by status and
/// background-task decisions.
pub(crate) fn include_pending_status(session: &mut Session) {
    let pending =
        bg_tasks::tasks_dir().and_then(|root| bg_tasks::has_pending_in(&root, &session.session_id));
    apply_pending_status(session, pending);
}

/// Includes registry state on Stop without touching the session file while
/// the hook is preparing its own session update.
pub(crate) fn include_pending_status_for_stop(session: &mut Session) {
    let pending = bg_tasks::tasks_dir().and_then(|root| {
        bg_tasks::scan_pending_in(&root, &session.session_id).map(|scan| scan.pending)
    });
    apply_pending_status(session, pending);
}

fn apply_pending_status(session: &mut Session, pending: Result<bool>) {
    match pending {
        Ok(true) => mark_pending_status(session),
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(
                event = "agent.bg_run.registry_read_failed",
                session = %session.session_id,
                error = %error,
            );
            mark_pending_status(session);
        }
    }
}

fn mark_pending_status(session: &mut Session) {
    session
        .pending_bg_task_ids
        .insert(BG_RUN_PENDING_TASK_MARKER.to_string());
}
