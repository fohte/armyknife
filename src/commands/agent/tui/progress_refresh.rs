use super::app::{App, ProgressScopeKind};
use super::event::EventHandler;

pub(super) fn start_progress_refresh(
    app: &mut App,
    event_handler: &EventHandler,
    session_id: String,
) {
    app.begin_progress_refresh(&session_id);
    match app.progress_fetch_scope(&session_id) {
        Some(ProgressScopeKind::OwnOnly) | None => {}
        Some(ProgressScopeKind::Direct) => {
            event_handler.start_progress_fetch(session_id, None, None);
        }
        Some(ProgressScopeKind::Task {
            task_id,
            session_ids,
        }) => {
            event_handler.start_progress_fetch(session_id, Some(task_id), Some(session_ids));
        }
    }
}
