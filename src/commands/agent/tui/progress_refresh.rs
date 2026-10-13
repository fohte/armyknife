use super::app::App;
use super::event::EventHandler;

pub(super) fn start_progress_refresh(
    app: &mut App,
    event_handler: &EventHandler,
    session_id: String,
) {
    app.begin_progress_refresh(&session_id);
    let (task_id, task_session_ids) = app.progress_fetch_task_scope(&session_id);
    event_handler.start_progress_fetch(session_id, task_id, task_session_ids);
}
