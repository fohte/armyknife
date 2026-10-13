use super::{create_test_app, create_test_session_at};
use crate::commands::agent::tui::progress::{
    FetchedDelegate, FetchedProgress, PrProgressRow, ProgressMark, ProgressRow, StageMarks,
    WorkProgressRow, build_progress_rows,
};
use crate::commands::agent::types::{Session, SessionStatus};
use crate::infra::github::{PrCiState, PrInfo, PrProgressInfo, PrState};
use crate::shared::config::AgentConfig;
use chrono::{DateTime, TimeZone, Utc};
use rstest::{fixture, rstest};

#[fixture]
fn timestamp() -> DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000, 0).single().unwrap()
}

#[fixture]
fn delegated_session(timestamp: DateTime<Utc>) -> Session {
    let mut session = create_test_session_at("delegate", timestamp);
    session.is_delegated = true;
    session.ancestor_session_ids = vec!["selected".to_string()];
    session
}

#[rstest]
fn stale_progress_response_keeps_the_new_selection_state(timestamp: DateTime<Utc>) {
    let mut app = create_test_app(vec![
        create_test_session_at("first", timestamp),
        create_test_session_at("second", timestamp),
    ]);
    let first = app.sessions[0].clone();
    app.progress_rows = build_progress_rows(&first, &[], &AgentConfig::default());
    app.progress_session_id = Some("first".to_string());
    app.progress_error = Some("pending progress state".to_string());
    app.progress_list_state.select(Some(0));
    app.progress_table_state.select(None);
    app.list_state.select(Some(2));
    let before = (
        app.progress_session_id.clone(),
        app.progress_rows.clone(),
        app.progress_error.clone(),
        app.progress_list_state.selected(),
        app.progress_table_state.selected(),
    );

    app.apply_progress_result(
        "first",
        Ok(FetchedProgress {
            selected_session_id: "first".to_string(),
            delegated: Vec::new(),
            implementation: None,
            github_error: None,
        }),
    );

    assert_eq!(
        (
            app.selected_session_id(),
            app.progress_session_id.as_deref(),
            app.progress_rows.clone(),
            app.progress_error.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        ),
        (
            Some("second"),
            before.0.as_deref(),
            before.1,
            before.2,
            before.3,
            before.4,
        ),
    );
}

#[rstest]
fn local_progress_refresh_preserves_the_selected_delegate_row(
    timestamp: DateTime<Utc>,
    delegated_session: Session,
) {
    let selected = create_test_session_at("selected", timestamp);
    let mut app = create_test_app(vec![selected.clone(), delegated_session.clone()]);
    app.update_row_order(&[None, Some("selected"), Some("delegate")]);
    app.list_state.select(Some(1));
    app.begin_progress_refresh("selected");
    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            delegated: vec![FetchedDelegate {
                session: delegated_session,
                checklist_item_id: None,
                pull_request: None,
                lookup_unavailable: false,
            }],
            implementation: None,
            github_error: None,
        }),
    );
    app.progress_list_state.select(Some(1));
    app.progress_table_state.select(Some(0));
    let updated_delegate = app
        .sessions
        .iter_mut()
        .find(|session| session.session_id == "delegate")
        .expect("delegate session exists");
    updated_delegate.status = SessionStatus::Ended;
    updated_delegate.label = Some("Updated delegated work".to_string());

    app.refresh_progress_from_sessions("selected");

    assert_eq!(
        (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        ),
        (
            vec![
                ProgressRow::Work(WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::PullRequest(PrProgressRow {
                    session_id: "delegate".to_string(),
                    title: "Updated delegated work".to_string(),
                    number: None,
                    url: None,
                    pane_id: None,
                    is_ended: true,
                    lookup_unavailable: false,
                    stages: StageMarks {
                        implementation: ProgressMark::Done,
                        diff_review: ProgressMark::NotStarted,
                        pr_body: ProgressMark::NotStarted,
                        ci: ProgressMark::NotStarted,
                        merge: ProgressMark::NotStarted,
                    },
                }),
            ],
            Some(1),
            Some(0),
        ),
    );
}

#[rstest]
fn shows_only_the_work_row_when_the_selected_session_has_no_delegates(timestamp: DateTime<Utc>) {
    let mut app = create_test_app(vec![create_test_session_at("selected", timestamp)]);

    app.begin_progress_refresh("selected");

    assert_eq!(
        (
            app.progress_session_id.clone(),
            app.progress_rows.clone(),
            app.progress_error.clone(),
        ),
        (
            Some("selected".to_string()),
            vec![ProgressRow::Work(WorkProgressRow {
                session_id: "selected".to_string(),
                role: "session".to_string(),
                mark: ProgressMark::Working,
                status_label: None,
                note: None,
                pane_id: None,
            })],
            None,
        ),
    );
}

#[rstest]
fn shows_local_delegates_before_pr_status_is_fetched(
    timestamp: DateTime<Utc>,
    mut delegated_session: Session,
) {
    let selected = create_test_session_at("selected", timestamp);
    delegated_session.label = Some("Local delegate".to_string());
    let mut app = create_test_app(vec![selected, delegated_session]);
    app.update_row_order(&[None, Some("selected"), Some("delegate")]);
    app.list_state.select(Some(1));

    app.begin_progress_refresh("selected");

    assert_eq!(
        app.progress_rows.clone(),
        vec![
            ProgressRow::Work(WorkProgressRow {
                session_id: "selected".to_string(),
                role: "session".to_string(),
                mark: ProgressMark::Working,
                status_label: None,
                note: None,
                pane_id: None,
            }),
            ProgressRow::PullRequest(PrProgressRow {
                session_id: "delegate".to_string(),
                title: "Local delegate".to_string(),
                number: None,
                url: None,
                pane_id: None,
                is_ended: false,
                lookup_unavailable: false,
                stages: StageMarks {
                    implementation: ProgressMark::Working,
                    diff_review: ProgressMark::NotStarted,
                    pr_body: ProgressMark::NotStarted,
                    ci: ProgressMark::NotStarted,
                    merge: ProgressMark::NotStarted,
                },
            }),
        ],
    );
}

#[rstest]
fn reuses_cached_progress_when_returning_to_a_session(
    timestamp: DateTime<Utc>,
    delegated_session: Session,
) {
    let selected = create_test_session_at("selected", timestamp);
    let mut app = create_test_app(vec![
        selected,
        delegated_session.clone(),
        create_test_session_at("other", timestamp),
    ]);
    app.update_row_order(&[None, Some("selected"), Some("other"), Some("delegate")]);
    app.list_state.select(Some(1));
    app.begin_progress_refresh("selected");

    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            delegated: vec![FetchedDelegate {
                session: delegated_session,
                checklist_item_id: None,
                pull_request: Some(PrProgressInfo {
                    pull_request: PrInfo {
                        number: 17,
                        title: "Example change".to_string(),
                        state: PrState::Open,
                        url: "https://example.test/pull/17".to_string(),
                    },
                    ci_state: PrCiState::Successful,
                }),
                lookup_unavailable: false,
            }],
            implementation: None,
            github_error: None,
        }),
    );

    app.list_state.select(Some(2));
    app.begin_progress_refresh("other");
    app.list_state.select(Some(1));
    app.begin_progress_refresh("selected");

    assert_eq!(
        (
            app.progress_session_id.as_deref(),
            app.progress_rows.clone(),
            app.progress_error.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        ),
        (
            Some("selected"),
            vec![
                ProgressRow::Work(WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::PullRequest(PrProgressRow {
                    session_id: "delegate".to_string(),
                    title: "Example change".to_string(),
                    number: Some(17),
                    url: Some("https://example.test/pull/17".to_string()),
                    pane_id: None,
                    is_ended: false,
                    lookup_unavailable: false,
                    stages: StageMarks {
                        implementation: ProgressMark::Done,
                        diff_review: ProgressMark::Done,
                        pr_body: ProgressMark::Done,
                        ci: ProgressMark::Done,
                        merge: ProgressMark::User,
                    },
                }),
            ],
            None,
            Some(0),
            None,
        ),
    );
}

#[rstest]
fn keeps_fetched_progress_for_ended_delegates_after_local_refresh(
    timestamp: DateTime<Utc>,
    mut delegated_session: Session,
) {
    let selected = create_test_session_at("selected", timestamp);
    delegated_session.status = SessionStatus::Ended;
    let mut app = create_test_app(vec![selected]);
    app.begin_progress_refresh("selected");
    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            delegated: vec![FetchedDelegate {
                session: delegated_session,
                checklist_item_id: None,
                pull_request: Some(PrProgressInfo {
                    pull_request: PrInfo {
                        number: 42,
                        title: "Finished change".to_string(),
                        state: PrState::Open,
                        url: "https://example.test/pull/42".to_string(),
                    },
                    ci_state: PrCiState::InProgress,
                }),
                lookup_unavailable: false,
            }],
            implementation: None,
            github_error: None,
        }),
    );

    app.refresh_progress_from_sessions("selected");

    assert_eq!(
        (
            app.progress_rows.clone(),
            app.progress_error.clone(),
            app.progress_session_id.as_deref(),
        ),
        (
            vec![
                ProgressRow::Work(WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::PullRequest(PrProgressRow {
                    session_id: "delegate".to_string(),
                    title: "Finished change".to_string(),
                    number: Some(42),
                    url: Some("https://example.test/pull/42".to_string()),
                    pane_id: None,
                    is_ended: true,
                    lookup_unavailable: false,
                    stages: StageMarks {
                        implementation: ProgressMark::Done,
                        diff_review: ProgressMark::Done,
                        pr_body: ProgressMark::Done,
                        ci: ProgressMark::Working,
                        merge: ProgressMark::Waiting,
                    },
                }),
            ],
            None,
            Some("selected"),
        ),
    );
}

#[rstest]
fn preserves_local_delegate_rows_and_selection_when_fetch_fails(
    timestamp: DateTime<Utc>,
    delegated_session: Session,
) {
    let mut app = create_test_app(vec![
        create_test_session_at("selected", timestamp),
        delegated_session,
    ]);
    app.update_row_order(&[None, Some("selected"), Some("delegate")]);
    app.list_state.select(Some(1));
    app.begin_progress_refresh("selected");
    app.progress_list_state.select(Some(1));
    app.progress_table_state.select(Some(0));

    app.apply_progress_result("selected", Err("progress fetch failed".to_string()));

    assert_eq!(
        (
            app.selected_session_id(),
            app.progress_session_id.as_deref(),
            app.progress_rows.clone(),
            app.progress_error.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        ),
        (
            Some("selected"),
            Some("selected"),
            vec![
                ProgressRow::Work(WorkProgressRow {
                    session_id: "selected".to_string(),
                    role: "session".to_string(),
                    mark: ProgressMark::Working,
                    status_label: None,
                    note: None,
                    pane_id: None,
                }),
                ProgressRow::PullRequest(PrProgressRow {
                    session_id: "delegate".to_string(),
                    title: "delegate".to_string(),
                    number: None,
                    url: None,
                    pane_id: None,
                    is_ended: false,
                    lookup_unavailable: false,
                    stages: StageMarks {
                        implementation: ProgressMark::Working,
                        diff_review: ProgressMark::NotStarted,
                        pr_body: ProgressMark::NotStarted,
                        ci: ProgressMark::NotStarted,
                        merge: ProgressMark::NotStarted,
                    },
                }),
            ],
            Some("progress fetch failed".to_string()),
            Some(1),
            Some(0),
        ),
    );
}

#[test]
fn falls_back_to_delegate_rows_when_checklist_progress_is_unavailable() {
    let timestamp = Utc.timestamp_opt(1_800_000_000, 0).single().unwrap();
    let selected = create_test_session_at("selected", timestamp);
    let mut delegate = create_test_session_at("delegate", timestamp);
    delegate.is_delegated = true;
    delegate.ancestor_session_ids = vec!["selected".to_string()];
    let fetched_delegate = FetchedDelegate {
        session: delegate.clone(),
        checklist_item_id: None,
        pull_request: None,
        lookup_unavailable: false,
    };
    let expected = build_progress_rows(
        &selected,
        std::slice::from_ref(&fetched_delegate),
        &AgentConfig::default(),
    );
    let mut app = create_test_app(vec![selected, delegate]);
    if app.selected_session_id() != Some("selected") {
        app.select_next();
    }

    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            delegated: vec![fetched_delegate],
            implementation: None,
            github_error: None,
        }),
    );

    assert_eq!(app.progress_rows, expected);
}
