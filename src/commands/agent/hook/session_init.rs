use std::path::Path;

use anyhow::Result;

use super::super::store::{self, SessionLock};
use super::super::types::{Session, SessionPrContext};

pub(super) struct InitializedSession {
    pub(super) lock: SessionLock,
    pub(super) existing_session: Option<Session>,
    pub(super) pr_context: Option<SessionPrContext>,
    pub(super) work_type_pinned: bool,
}

pub(super) fn load_or_capture(
    sessions_dir: &Path,
    session_id: &str,
    cwd: &Path,
    capture_pr_context: impl FnOnce(&Path) -> Option<SessionPrContext>,
    determine_work_type_pin: impl FnOnce() -> bool,
) -> Result<InitializedSession> {
    let lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let existing_session = lock.load()?;
    if existing_session.is_some() {
        return Ok(InitializedSession {
            lock,
            existing_session,
            pr_context: None,
            work_type_pinned: false,
        });
    }

    drop(lock);
    let pr_context = capture_pr_context(cwd);
    let work_type_pinned = determine_work_type_pin();
    let lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let existing_session = lock.load()?;

    Ok(InitializedSession {
        lock,
        existing_session,
        pr_context,
        work_type_pinned,
    })
}
