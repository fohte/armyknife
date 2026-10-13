use super::app::App;
use super::event::EventHandler;

pub(super) fn start_progress_refresh(
    app: &mut App,
    event_handler: &EventHandler,
    session_id: String,
) {
    app.begin_progress_refresh(&session_id);
    let task_id = app
        .task_by_session
        .get(&session_id)
        .map(|task| task.task_id.clone());
    let task_session_ids = task_id
        .as_deref()
        .map(|task_id| app.progress_task_session_ids(task_id));
    event_handler.start_progress_fetch(session_id, task_id, task_session_ids);
}
