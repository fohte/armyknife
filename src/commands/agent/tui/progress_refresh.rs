use super::ProgressRefreshSchedule;
use super::app::App;
use super::event::EventHandler;
use super::tq_snapshot::TqSnapshot;

pub(super) fn start_pending_progress_refresh(
    schedule: &mut ProgressRefreshSchedule,
    start: impl FnOnce(String) -> bool,
) {
    let Some(session_id) = schedule.start_pending() else {
        return;
    };
    if !start(session_id) {
        schedule.fetch_completed();
    }
}

pub(super) fn apply_tq_snapshot_and_schedule_progress_refresh(
    app: &mut App,
    schedule: &mut ProgressRefreshSchedule,
    snapshot: TqSnapshot,
) -> bool {
    let scope_changed = app.set_tq_snapshot(snapshot);
    if scope_changed {
        let session_id = app.selected_session_id().map(str::to_owned);
        if let Some(session_id) = session_id.as_deref() {
            app.refresh_progress_from_sessions(session_id);
        }
        schedule.request(session_id);
    }
    scope_changed
}

pub(super) fn start_progress_refresh(
    app: &mut App,
    event_handler: &EventHandler,
    session_id: String,
) -> bool {
    app.begin_progress_refresh(&session_id);
    let Some(fetch_scope) = app
        .progress_scope_for_session(&session_id)
        .and_then(|scope| scope.fetch_scope())
    else {
        return false;
    };
    event_handler.start_progress_fetch(session_id, fetch_scope);
    true
}
