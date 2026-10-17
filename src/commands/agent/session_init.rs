use std::path::Path;

use anyhow::Result;

use super::store::{self, SessionLock};
use super::types::Session;

pub(crate) struct InitializedSession<T> {
    pub(crate) lock: SessionLock,
    pub(crate) existing_session: Option<Session>,
    pub(crate) captured: Option<T>,
}

pub(crate) fn load_or_capture<T>(
    sessions_dir: &Path,
    session_id: &str,
    cwd: &Path,
    capture: impl FnOnce(&Path) -> T,
) -> Result<InitializedSession<T>> {
    let lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let existing_session = lock.load()?;
    if existing_session.is_some() {
        return Ok(InitializedSession::<T> {
            lock,
            existing_session,
            captured: None,
        });
    }

    drop(lock);
    let captured = capture(cwd);
    let lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let existing_session = lock.load()?;

    Ok(InitializedSession {
        lock,
        existing_session,
        captured: Some(captured),
    })
}
