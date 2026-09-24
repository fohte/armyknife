use anyhow::Result;

use super::bg_tasks;
use super::store;
use super::types::Session;

/// Loads one session with `a agent bg run` registry state included for
/// presentation and background-task decisions.
pub(crate) fn load_session_with_bg_run_status(session_id: &str) -> Result<Option<Session>> {
    let Some(mut session) = store::load_session(session_id)? else {
        return Ok(None);
    };
    bg_tasks::include_pending_status(&mut session);
    Ok(Some(session))
}

/// Lists active sessions with `a agent bg run` registry state included for
/// presentation and background-task decisions.
pub(crate) fn list_sessions_with_bg_run_status() -> Result<Vec<Session>> {
    let mut sessions = store::list_sessions()?;
    for session in &mut sessions {
        bg_tasks::include_pending_status(session);
    }
    Ok(sessions)
}

/// Rewrites the session file without changing its contents so file watchers
/// reload the derived status when an external background task changes state.
pub(crate) fn touch_session(session_id: &str) -> Result<()> {
    let sessions_dir = store::sessions_dir()?;
    let lock = store::lock_session_for_update(&sessions_dir, session_id)?;
    if let Some(session) = lock.load()? {
        lock.save(&session)?;
    }
    Ok(())
}
