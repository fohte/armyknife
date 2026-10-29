use std::time::{Duration, Instant};
use std::{collections::HashMap, path::PathBuf};

use chrono::Utc;

use super::app::App;
use super::event::EventHandler;
use super::progress::ProgressFetchState;
use super::tq_snapshot::TqSnapshot;
use crate::commands::agent::types::{Session, SessionPrContext};

pub(super) const PROGRESS_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkerPrInputSnapshot(HashMap<String, (PathBuf, Option<SessionPrContext>)>);

impl WorkerPrInputSnapshot {
    pub(super) fn new(sessions: &[Session]) -> Self {
        Self(worker_pr_inputs(sessions))
    }

    pub(super) fn update(&mut self, sessions: &[Session]) -> bool {
        let inputs = worker_pr_inputs(sessions);
        if self.0 == inputs {
            return false;
        }
        self.0 = inputs;
        true
    }
}

fn worker_pr_inputs(sessions: &[Session]) -> HashMap<String, (PathBuf, Option<SessionPrContext>)> {
    sessions
        .iter()
        .map(|session| {
            (
                session.session_id.clone(),
                (session.cwd.clone(), session.pr_context.clone()),
            )
        })
        .collect()
}

pub(super) struct WorkerPrRefreshSchedule {
    in_flight: bool,
    pending: bool,
    next_periodic_refresh: Instant,
}

impl WorkerPrRefreshSchedule {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            in_flight: false,
            pending: false,
            next_periodic_refresh: now + PROGRESS_REFRESH_INTERVAL,
        }
    }

    pub(super) fn request(&mut self) {
        self.pending = true;
    }

    pub(super) fn periodic_refresh_due(&self, now: Instant) -> bool {
        now >= self.next_periodic_refresh
    }

    pub(super) fn fetch_completed(&mut self) {
        self.in_flight = false;
    }

    pub(super) fn start_pending(&mut self, now: Instant) -> bool {
        if self.in_flight || !self.pending {
            return false;
        }
        self.pending = false;
        self.in_flight = true;
        self.next_periodic_refresh = now + PROGRESS_REFRESH_INTERVAL;
        true
    }
}

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

    pub(super) fn periodic_refresh_due(&self, now: Instant) -> bool {
        now >= self.next_periodic_refresh
    }

    fn schedule_next_periodic(&mut self, now: Instant, delay: Duration) {
        self.next_periodic_refresh = now + delay;
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

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ProgressRefreshStart {
    Started,
    Fresh(Duration),
    MissingScope,
}

pub(super) fn start_pending_progress_refresh(
    schedule: &mut ProgressRefreshSchedule,
    now: Instant,
    start: impl FnOnce(String) -> ProgressRefreshStart,
) {
    let Some(session_id) = schedule.start_pending() else {
        return;
    };
    match start(session_id) {
        ProgressRefreshStart::Started => {
            schedule.schedule_next_periodic(now, PROGRESS_REFRESH_INTERVAL);
        }
        ProgressRefreshStart::Fresh(wait) => {
            schedule.schedule_next_periodic(now, wait);
            schedule.fetch_completed();
        }
        ProgressRefreshStart::MissingScope => {
            schedule.schedule_next_periodic(now, PROGRESS_REFRESH_INTERVAL);
            schedule.fetch_completed();
        }
    }
}

pub(super) fn apply_tq_snapshot_and_schedule_progress_refresh(
    app: &mut App,
    schedule: &mut ProgressRefreshSchedule,
    snapshot: TqSnapshot,
) -> bool {
    let scope_changed = app.set_tq_snapshot(snapshot);
    if scope_changed {
        let session_id = app.progress_inline_source_id.clone();
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
) -> ProgressRefreshStart {
    app.begin_progress_refresh(&session_id);
    let Some(fetch_scope) = app
        .progress_scope_for_session(&session_id)
        .and_then(|scope| scope.fetch_scope())
    else {
        app.progress_fetch_states
            .insert(session_id, ProgressFetchState::Ready);
        return ProgressRefreshStart::MissingScope;
    };
    let now = Utc::now();
    if let Some(wait) = app.progress_refresh_wait(&session_id, now, PROGRESS_REFRESH_INTERVAL)
        && !wait.is_zero()
    {
        app.progress_fetch_states
            .entry(session_id)
            .or_insert(ProgressFetchState::Ready);
        return ProgressRefreshStart::Fresh(wait);
    }
    app.mark_progress_refresh_started(&session_id, now);
    app.progress_fetch_states
        .insert(session_id.clone(), ProgressFetchState::Loading);
    event_handler.start_progress_fetch(session_id, fetch_scope);
    ProgressRefreshStart::Started
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{Engine, SessionStatus};
    use chrono::{TimeZone, Utc};
    use std::collections::BTreeSet;

    #[test]
    fn worker_pr_refresh_coalesces_requests_while_a_fetch_is_running() {
        let now = Instant::now();
        let mut schedule = WorkerPrRefreshSchedule::new(now);
        schedule.request();

        assert_eq!(
            (
                schedule.start_pending(now),
                schedule.start_pending(now),
                schedule.periodic_refresh_due(now),
            ),
            (true, false, false),
        );
        schedule.request();
        let while_in_flight = schedule.start_pending(now);

        schedule.fetch_completed();
        let after_completion = schedule.start_pending(now);

        assert_eq!((while_in_flight, after_completion), (false, true));
    }

    #[test]
    fn worker_pr_inputs_ignore_session_activity_but_refresh_for_branch_changes() {
        let timestamp = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).single().unwrap();
        let mut session = Session {
            archived_at: None,
            pr_context: Some(SessionPrContext {
                owner: "sample-owner".to_string(),
                repo: "sample-repo".to_string(),
                branch: "sample-branch".to_string(),
                checklist_item_id: Some("item-a".to_string()),
            }),
            session_id: "worker".to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/project/worker"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: timestamp,
            updated_at: timestamp,
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        };
        let mut snapshot = WorkerPrInputSnapshot::new(&[session.clone()]);
        session.updated_at += chrono::Duration::seconds(1);
        session.last_message = Some("Working on the branch".to_string());
        let activity_only = snapshot.update(&[session.clone()]);
        session.pr_context.as_mut().expect("context exists").branch = "next-branch".to_string();
        let branch_changed = snapshot.update(&[session]);

        assert_eq!((activity_only, branch_changed), (false, true));
    }
}
