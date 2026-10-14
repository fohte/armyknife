use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use rstest::{fixture, rstest};

use super::super::App;
use super::super::progress::CachedProgress;
use super::{create_test_app, create_test_session_at};
use crate::commands::agent::tui::progress::{
    FetchedDelegate, FetchedProgress, PrProgressRow, ProgressFetchScope, ProgressMark, ProgressRow,
    StageMarks, WorkProgressRow, build_task_progress_rows,
};
use crate::commands::agent::tui::session_rows::SessionTask;
use crate::commands::agent::tui::tq_snapshot::TqSnapshot;
use crate::commands::agent::types::Session;
use crate::infra::github::{PrCiState, PrInfo, PrProgressInfo, PrState};
use crate::shared::config::AgentConfig;

#[fixture]
fn timestamp() -> DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000, 0).single().unwrap()
}

#[fixture]
fn app_with_task(timestamp: DateTime<Utc>) -> App {
    let sessions = vec![
        create_test_session_at("session-a", timestamp),
        create_test_session_at("session-b", timestamp + ChronoDuration::seconds(1)),
    ];
    let mut app = create_test_app(sessions);
    app.set_tq_snapshot(TqSnapshot::new(HashMap::from([
        ("session-a".to_string(), vec![linked_task()]),
        ("session-b".to_string(), vec![linked_task()]),
    ])));
    select_session(&mut app, "session-b");
    app
}

#[rstest]
fn cached_progress_is_restored_for_another_session_linked_to_the_same_task(
    mut app_with_task: App,
    timestamp: DateTime<Utc>,
) {
    let cached_rows = task_rows();
    app_with_task.restore_progress_cache(HashMap::from([(
        "task-placeholder".to_string(),
        cached_progress(timestamp, cached_rows.clone()),
    )]));

    app_with_task.begin_progress_refresh("session-b");

    assert_eq!(
        (
            app_with_task.progress_rows.clone(),
            app_with_task.progress_list_state.selected(),
        ),
        (cached_rows, Some(1)),
    );
}

#[rstest]
#[case::cache_is_fresh(29, false)]
#[case::cache_is_due(30, true)]
fn progress_refresh_uses_the_task_cache_age(
    mut app_with_task: App,
    timestamp: DateTime<Utc>,
    #[case] age_seconds: i64,
    #[case] expected: bool,
) {
    app_with_task.restore_progress_cache(HashMap::from([(
        "task-placeholder".to_string(),
        cached_progress(timestamp, task_rows()),
    )]));

    assert_eq!(
        app_with_task.progress_refresh_due(
            "session-b",
            timestamp + ChronoDuration::seconds(age_seconds),
            Duration::from_secs(30),
        ),
        expected,
    );
}

#[rstest]
fn reapplying_the_same_progress_result_keeps_the_rendered_rows_and_selection(
    timestamp: DateTime<Utc>,
) {
    let selected = create_test_session_at("session-root", timestamp);
    let mut delegate = create_test_session_at("session-delegate", timestamp);
    delegate.is_delegated = true;
    delegate.ancestor_session_ids = vec!["session-root".to_string()];
    let mut app = create_test_app(vec![selected.clone(), delegate.clone()]);
    select_session(&mut app, "session-root");
    let expected_rows = build_task_progress_rows(
        std::slice::from_ref(&selected),
        &[fetched_delegate(delegate.clone())],
        &AgentConfig::default(),
    );

    app.apply_progress_result(
        "session-root",
        Ok(fetched_progress("session-root", delegate.clone())),
    );
    app.progress_list_state.select(Some(1));
    app.apply_progress_result(
        "session-root",
        Ok(fetched_progress("session-root", delegate)),
    );

    assert_eq!(
        (
            app.progress_rows.clone(),
            app.progress_list_state.selected()
        ),
        (expected_rows, Some(1)),
    );
}

fn cached_progress(timestamp: DateTime<Utc>, rows: Vec<ProgressRow>) -> CachedProgress {
    CachedProgress {
        rows,
        error: None,
        implementation: None,
        request_scope: Some(ProgressFetchScope {
            task_id: Some("task-placeholder".to_string()),
            session_ids: Some(vec!["session-a".to_string(), "session-b".to_string()]),
        }),
        delegated_sessions: Vec::new(),
        fetched_at: Some(timestamp),
        last_attempt_at: Some(timestamp),
    }
}

fn task_rows() -> Vec<ProgressRow> {
    vec![
        ProgressRow::Work(WorkProgressRow {
            session_id: "session-a".to_string(),
            role: "implementation".to_string(),
            mark: ProgressMark::Working,
            status_label: None,
            note: None,
            pane_id: None,
        }),
        ProgressRow::Work(WorkProgressRow {
            session_id: "session-b".to_string(),
            role: "implementation".to_string(),
            mark: ProgressMark::Working,
            status_label: None,
            note: None,
            pane_id: None,
        }),
        ProgressRow::ImplementationHeader {
            merged_pr_count: 0,
            total_pr_count: 1,
            collapsed: false,
        },
        ProgressRow::ChecklistPullRequest {
            batch_id: "batch-placeholder".to_string(),
            row: PrProgressRow {
                session_id: "delegate-placeholder".to_string(),
                title: "Placeholder change".to_string(),
                number: Some(7),
                url: Some("https://example.test/pull/7".to_string()),
                pane_id: None,
                is_ended: false,
                lookup_unavailable: false,
                stages: StageMarks {
                    implementation: ProgressMark::Done,
                    diff_review: ProgressMark::User,
                    pr_body: ProgressMark::Working,
                    ci: ProgressMark::Waiting,
                    merge: ProgressMark::NotStarted,
                },
            },
        },
    ]
}

fn fetched_progress(selected_session_id: &str, delegate: Session) -> FetchedProgress {
    FetchedProgress {
        selected_session_id: selected_session_id.to_string(),
        request_scope: Some(ProgressFetchScope {
            task_id: None,
            session_ids: None,
        }),
        delegated: vec![fetched_delegate(delegate)],
        implementation: None,
        github_error: None,
    }
}

fn fetched_delegate(session: Session) -> FetchedDelegate {
    FetchedDelegate {
        session,
        checklist_item_id: None,
        pull_request: Some(PrProgressInfo {
            pull_request: PrInfo {
                number: 7,
                title: "Placeholder change".to_string(),
                state: PrState::Open,
                url: "https://example.test/pull/7".to_string(),
            },
            ci_state: PrCiState::InProgress,
        }),
        lookup_unavailable: false,
    }
}

fn linked_task() -> SessionTask {
    SessionTask {
        task_id: "task-placeholder".to_string(),
        task_number: 7,
        task_title: "Placeholder task".to_string(),
        parent_task_id: None,
        is_closed: false,
    }
}

fn select_session(app: &mut App, session_id: &str) {
    let index = app
        .row_sessions
        .iter()
        .position(|session_index| {
            session_index.is_some_and(|index| app.sessions[index].session_id == session_id)
        })
        .unwrap();
    app.list_state.select(Some(index));
}
