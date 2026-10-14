use std::time::{Duration, Instant};

use chrono::Utc;

use super::app::App;
use super::event::EventHandler;
use super::tq_snapshot::TqSnapshot;

pub(super) const PROGRESS_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

pub(super) struct ProgressRefreshSchedule {
    pub(super) in_flight: bool,
    pub(super) pending_session_id: Option<String>,
    next_periodic_refresh: Instant,
}

impl ProgressRefreshSchedule {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            in_flight: false,
            pending_session_id: None,
            next_periodic_refresh: now + PROGRESS_REFRESH_INTERVAL,
        }
    }

    pub(super) fn request(&mut self, session_id: Option<String>) {
        self.pending_session_id = session_id;
    }

    pub(super) fn periodic_refresh_due(&mut self, now: Instant) -> bool {
        if now < self.next_periodic_refresh {
            return false;
        }
        self.next_periodic_refresh = now + PROGRESS_REFRESH_INTERVAL;
        true
    }

    pub(super) fn fetch_completed(&mut self) {
        self.in_flight = false;
    }

    pub(super) fn start_pending(&mut self) -> Option<String> {
        if self.in_flight {
            return None;
        }
        let session_id = self.pending_session_id.take()?;
        self.in_flight = true;
        Some(session_id)
    }
}

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
    let now = Utc::now();
    if !app.progress_refresh_due(&session_id, now, PROGRESS_REFRESH_INTERVAL) {
        return false;
    }
    app.mark_progress_refresh_started(&session_id, now);
    event_handler.start_progress_fetch(session_id, fetch_scope);
    true
}
