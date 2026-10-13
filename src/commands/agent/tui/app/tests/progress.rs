use super::{create_test_app, create_test_session_at};
use crate::commands::agent::tui::progress::{
    FetchedDelegate, FetchedProgress, PrProgressRow, ProgressMark, ProgressRow, StageMarks,
    WorkProgressRow, build_progress_rows,
};
use crate::commands::agent::types::SessionStatus;
use crate::shared::config::AgentConfig;
use chrono::{TimeZone, Utc};

#[test]
fn stale_progress_response_keeps_the_new_selection_state() {
    let timestamp = Utc.timestamp_opt(1_800_000_000, 0).single().unwrap();
    let mut app = create_test_app(vec![
        create_test_session_at("first", timestamp),
        create_test_session_at("second", timestamp),
    ]);
    let first = app.sessions[0].clone();
    app.progress_rows = build_progress_rows(&first, &[], &AgentConfig::default());
    app.progress_session_id = Some("first".to_string());
    app.progress_loading = true;
    app.progress_error = Some("pending progress state".to_string());
    app.progress_list_state.select(Some(0));
    app.progress_table_state.select(None);
    app.list_state.select(Some(2));
    let before = (
        app.progress_session_id.clone(),
        app.progress_rows.clone(),
        app.progress_loading,
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
            app.progress_loading,
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
            before.5,
        ),
    );
}

#[test]
fn local_progress_refresh_preserves_the_selected_delegate_row() {
    let timestamp = Utc.timestamp_opt(1_800_000_000, 0).single().unwrap();
    let selected = create_test_session_at("selected", timestamp);
    let mut delegate = create_test_session_at("delegate", timestamp);
    delegate.is_delegated = true;
    delegate.ancestor_session_ids = vec!["selected".to_string()];
    let mut app = create_test_app(vec![selected.clone(), delegate.clone()]);
    if app.selected_session_id() != Some("selected") {
        app.select_next();
    }
    app.progress_session_id = Some("selected".to_string());
    app.progress_rows = build_progress_rows(
        &selected,
        &[FetchedDelegate {
            session: delegate,
            checklist_item_id: None,
            pull_request: None,
            lookup_unavailable: false,
        }],
        &AgentConfig::default(),
    );
    app.progress_list_state.select(Some(1));
    app.progress_table_state.select(Some(0));
    app.progress_loading = true;
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
            app.progress_loading,
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
                },),
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
            true,
        ),
    );
}
