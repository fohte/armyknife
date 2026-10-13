use std::collections::HashMap;

use super::super::App;
use super::{create_test_app, create_test_session_at};
use crate::commands::agent::tui::progress::{
    FetchedDelegate, FetchedProgress, PrProgressRow, ProgressMark, ProgressRow, StageMarks,
    WorkProgressRow, build_progress_rows,
};
use crate::commands::agent::tui::progress_batches::{
    ImplementationPlan, build_implementation_plan,
};
use crate::commands::agent::tui::session_rows::SessionTask;
use crate::commands::agent::tui::tq_snapshot::TqSnapshot;
use crate::commands::agent::types::{Session, SessionStatus};
use crate::infra::github::{PrCiState, PrInfo, PrProgressInfo, PrState};
use crate::infra::tq::{TqChecklist, TqChecklistItem, TqGithubLink};
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

#[fixture]
fn selected_task_session(timestamp: DateTime<Utc>) -> Session {
    create_test_session_at("selected", timestamp)
}

#[fixture]
fn linked_checklist_plan() -> ImplementationPlan {
    let checklist = TqChecklist {
        id: "checklist-a".to_string(),
        name: "実装".to_string(),
        items: vec![TqChecklistItem {
            id: "batch-a".to_string(),
            content: "Batch 1".to_string(),
            checked_at: None,
            github_link_id: None,
            children: vec![TqChecklistItem {
                id: "item-a".to_string(),
                content: "PR 1: Placeholder change".to_string(),
                checked_at: None,
                github_link_id: Some("link-a".to_string()),
                children: Vec::new(),
            }],
        }],
    };
    let github_link = TqGithubLink {
        id: "link-a".to_string(),
        kind: Some("pull_request".to_string()),
        owner: Some("example-org".to_string()),
        repo: Some("example-project".to_string()),
        number: Some(23),
        title: Some("Placeholder change".to_string()),
        url: Some("https://example.test/pull/23".to_string()),
        state: Some("open".to_string()),
    };
    build_implementation_plan(&checklist, &[github_link], &[], &HashMap::new(), true)
}

fn expected_unavailable_checklist_rows(session_id: &str) -> Vec<ProgressRow> {
    vec![
        ProgressRow::Work(WorkProgressRow {
            session_id: session_id.to_string(),
            role: "session".to_string(),
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
            batch_id: "batch-a".to_string(),
            row: PrProgressRow {
                session_id: "checklist-item:item-a".to_string(),
                title: "Placeholder change".to_string(),
                number: Some(23),
                url: Some("https://example.test/pull/23".to_string()),
                pane_id: None,
                is_ended: true,
                lookup_unavailable: true,
                stages: StageMarks {
                    implementation: ProgressMark::Done,
                    diff_review: ProgressMark::NotStarted,
                    pr_body: ProgressMark::NotStarted,
                    ci: ProgressMark::NotStarted,
                    merge: ProgressMark::NotStarted,
                },
            },
        },
    ]
}

fn expected_work_row(session_id: &str, role: &str) -> ProgressRow {
    ProgressRow::Work(WorkProgressRow {
        session_id: session_id.to_string(),
        role: role.to_string(),
        mark: ProgressMark::Working,
        status_label: None,
        note: None,
        pane_id: None,
    })
}

fn expected_unlinked_delegate_row(session_id: &str) -> ProgressRow {
    ProgressRow::PullRequest(PrProgressRow {
        session_id: session_id.to_string(),
        title: session_id.to_string(),
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
    })
}

fn task_link(task_id: &str) -> SessionTask {
    SessionTask {
        task_id: task_id.to_string(),
        task_number: 10,
        task_title: "Example task".to_string(),
        parent_task_id: None,
        is_closed: false,
    }
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
fn task_progress_shows_all_linked_sessions_and_their_delegate_descendants(
    timestamp: DateTime<Utc>,
) {
    let mut same_task_child = create_test_session_at("same-task-child", timestamp);
    same_task_child.ancestor_session_ids = vec!["selected".to_string()];
    let mut selected = create_test_session_at("selected", timestamp);
    selected.work_type = Some("design".to_string());
    let mut other_task_child = create_test_session_at("other-task-child", timestamp);
    other_task_child.ancestor_session_ids = vec!["selected".to_string()];
    let mut delegate = create_test_session_at("delegate", timestamp);
    delegate.is_delegated = true;
    delegate.ancestor_session_ids = vec!["selected".to_string(), "same-task-child".to_string()];
    let mut nested_delegate = create_test_session_at("nested-delegate", timestamp);
    nested_delegate.is_delegated = true;
    nested_delegate.ancestor_session_ids = vec![
        "selected".to_string(),
        "same-task-child".to_string(),
        "delegate".to_string(),
    ];
    let mut other_task_delegate = create_test_session_at("other-task-delegate", timestamp);
    other_task_delegate.is_delegated = true;
    other_task_delegate.ancestor_session_ids =
        vec!["selected".to_string(), "other-task-child".to_string()];
    let unrelated_same_task = create_test_session_at("unrelated-same-task", timestamp);
    let mut app = create_test_app(vec![
        same_task_child,
        selected,
        other_task_child,
        delegate.clone(),
        nested_delegate.clone(),
        other_task_delegate,
        unrelated_same_task,
    ]);
    app.update_row_order(&[
        None,
        Some("same-task-child"),
        Some("selected"),
        Some("other-task-child"),
        Some("delegate"),
        Some("nested-delegate"),
        Some("other-task-delegate"),
        Some("unrelated-same-task"),
    ]);
    app.list_state.select(Some(2));
    app.set_tq_snapshot(TqSnapshot::new(HashMap::from([
        ("selected".to_string(), vec![task_link("task-a")]),
        (
            "same-task-child".to_string(),
            vec![task_link("task-b"), task_link("task-a")],
        ),
        ("other-task-child".to_string(), vec![task_link("task-b")]),
        ("unrelated-same-task".to_string(), vec![task_link("task-a")]),
    ])));

    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            delegated: vec![
                FetchedDelegate {
                    session: delegate,
                    checklist_item_id: None,
                    pull_request: None,
                    lookup_unavailable: false,
                },
                FetchedDelegate {
                    session: nested_delegate,
                    checklist_item_id: None,
                    pull_request: None,
                    lookup_unavailable: false,
                },
            ],
            implementation: None,
            github_error: None,
        }),
    );

    assert_eq!(
        (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_list_state
                .selected()
                .and_then(|index| app.progress_rows.get(index))
                .and_then(ProgressRow::selection_key),
            app.progress_table_state.selected(),
        ),
        (
            vec![
                expected_work_row("same-task-child", "session"),
                expected_work_row("selected", "design"),
                expected_work_row("unrelated-same-task", "session"),
                expected_unlinked_delegate_row("delegate"),
                expected_unlinked_delegate_row("nested-delegate"),
            ],
            Some(1),
            Some("selected"),
            None,
        ),
    );
}

#[rstest]
fn task_progress_keeps_the_checklist_after_all_work_rows(
    timestamp: DateTime<Utc>,
    linked_checklist_plan: ImplementationPlan,
) {
    let selected = create_test_session_at("selected", timestamp);
    let mut same_task = create_test_session_at("same-task", timestamp);
    same_task.ancestor_session_ids = vec!["selected".to_string()];
    let mut app = create_test_app(vec![selected, same_task]);
    app.update_row_order(&[None, Some("selected"), Some("same-task")]);
    app.list_state.select(Some(1));
    app.set_tq_snapshot(TqSnapshot::new(HashMap::from([
        ("selected".to_string(), vec![task_link("task-a")]),
        ("same-task".to_string(), vec![task_link("task-a")]),
    ])));

    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            delegated: Vec::new(),
            implementation: Some(linked_checklist_plan),
            github_error: None,
        }),
    );

    assert_eq!(
        (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        ),
        (
            vec![
                expected_work_row("selected", "session"),
                expected_work_row("same-task", "session"),
                ProgressRow::ImplementationHeader {
                    merged_pr_count: 0,
                    total_pr_count: 1,
                    collapsed: false,
                },
                expected_unavailable_checklist_rows("selected")[2].clone(),
            ],
            Some(0),
            None,
        ),
    );
}

#[rstest]
fn project_manager_progress_shows_only_its_work_row_when_children_share_task(
    timestamp: DateTime<Utc>,
    linked_checklist_plan: ImplementationPlan,
) {
    let mut project_manager = create_test_session_at("project-manager", timestamp);
    project_manager.work_type = Some("project-manager".to_string());
    let mut child_task = create_test_session_at("child-task", timestamp);
    child_task.ancestor_session_ids = vec!["project-manager".to_string()];
    let mut child_delegate = create_test_session_at("child-delegate", timestamp);
    child_delegate.is_delegated = true;
    child_delegate.ancestor_session_ids =
        vec!["project-manager".to_string(), "child-task".to_string()];
    let mut app = create_test_app(vec![project_manager, child_task, child_delegate.clone()]);
    app.update_row_order(&[
        None,
        Some("project-manager"),
        Some("child-task"),
        Some("child-delegate"),
    ]);
    app.list_state.select(Some(1));
    app.set_tq_snapshot(TqSnapshot::new(HashMap::from([
        (
            "project-manager".to_string(),
            vec![task_link("parent-task")],
        ),
        ("child-task".to_string(), vec![task_link("parent-task")]),
    ])));

    assert_eq!(
        app.progress_fetch_task_scope("project-manager"),
        (None, Some(Vec::new())),
    );

    app.apply_progress_result(
        "project-manager",
        Ok(FetchedProgress {
            selected_session_id: "project-manager".to_string(),
            delegated: vec![FetchedDelegate {
                session: child_delegate,
                checklist_item_id: None,
                pull_request: None,
                lookup_unavailable: false,
            }],
            implementation: Some(linked_checklist_plan),
            github_error: None,
        }),
    );

    let expected = (
        vec![expected_work_row("project-manager", "project-manager")],
        Some(0),
        None,
    );
    assert_eq!(
        (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        ),
        expected.clone(),
    );

    app.refresh_progress_from_sessions("project-manager");

    assert_eq!(
        (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        ),
        expected,
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
fn initial_focus_selects_the_delegated_pr_row(
    selected_task_session: Session,
    delegated_session: Session,
) {
    let mut app = App::with_initial_selection(
        vec![selected_task_session, delegated_session],
        AgentConfig::default(),
        Some("delegate"),
        None,
    );

    app.begin_progress_refresh("selected");

    assert_eq!(
        (
            app.selected_session_id(),
            app.progress_focused,
            app.progress_list_state.selected(),
            app.progress_list_state
                .selected()
                .and_then(|index| app.progress_rows.get(index))
                .and_then(ProgressRow::selection_key),
        ),
        (Some("selected"), true, Some(1), Some("delegate")),
    );
}

#[rstest]
fn nested_delegate_focus_selects_the_direct_delegate_progress_row(timestamp: DateTime<Utc>) {
    let root = create_test_session_at("root", timestamp);
    let mut task = create_test_session_at("task", timestamp);
    task.ancestor_session_ids = vec![root.session_id.clone()];
    let mut intermediate_delegate = create_test_session_at("intermediate-delegate", timestamp);
    intermediate_delegate.is_delegated = true;
    intermediate_delegate.ancestor_session_ids =
        vec![root.session_id.clone(), task.session_id.clone()];
    let mut delegate = create_test_session_at("delegate", timestamp);
    delegate.is_delegated = true;
    delegate.ancestor_session_ids = vec![
        root.session_id.clone(),
        task.session_id.clone(),
        intermediate_delegate.session_id.clone(),
    ];
    let mut app = App::with_initial_selection(
        vec![root, task, intermediate_delegate, delegate],
        AgentConfig::default(),
        Some("delegate"),
        None,
    );

    app.begin_progress_refresh("task");

    assert_eq!(
        (
            app.selected_session_id(),
            app.progress_focused,
            app.progress_list_state
                .selected()
                .and_then(|index| app.progress_rows.get(index))
                .and_then(ProgressRow::selection_key),
        ),
        (Some("task"), true, Some("intermediate-delegate")),
    );
}

#[rstest]
fn initial_focus_expands_and_selects_a_merged_checklist_pr_row(
    selected_task_session: Session,
    delegated_session: Session,
) {
    let mut app = App::with_initial_selection(
        vec![selected_task_session, delegated_session.clone()],
        AgentConfig::default(),
        Some("delegate"),
        None,
    );
    app.begin_progress_refresh("selected");
    let checklist = TqChecklist {
        id: "checklist-a".to_string(),
        name: "実装".to_string(),
        items: vec![TqChecklistItem {
            id: "batch-a".to_string(),
            content: "Batch 1".to_string(),
            checked_at: None,
            github_link_id: None,
            children: vec![TqChecklistItem {
                id: "item-a".to_string(),
                content: "PR 1: Sample change".to_string(),
                checked_at: None,
                github_link_id: Some("link-a".to_string()),
                children: Vec::new(),
            }],
        }],
    };
    let github_link = TqGithubLink {
        id: "link-a".to_string(),
        kind: Some("pull_request".to_string()),
        owner: Some("example-org".to_string()),
        repo: Some("example-project".to_string()),
        number: Some(23),
        title: Some("Sample change".to_string()),
        url: Some("https://example.test/pull/23".to_string()),
        state: Some("merged".to_string()),
    };
    let fetched_delegates = vec![FetchedDelegate {
        session: delegated_session,
        checklist_item_id: Some("item-a".to_string()),
        pull_request: None,
        lookup_unavailable: false,
    }];
    let implementation = build_implementation_plan(
        &checklist,
        &[github_link],
        &fetched_delegates,
        &HashMap::new(),
        false,
    );

    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            delegated: fetched_delegates,
            implementation: Some(implementation),
            github_error: None,
        }),
    );

    assert_eq!(
        (
            app.progress_list_state.selected(),
            app.progress_list_state
                .selected()
                .and_then(|index| app.progress_rows.get(index))
                .and_then(ProgressRow::selection_key),
            app.progress_implementation_expanded,
            app.progress_batch_expanded.get("batch-a").copied(),
        ),
        (Some(2), Some("delegate"), true, Some(true)),
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

#[rstest]
fn restores_checklist_progress_when_returning_to_a_session(
    timestamp: DateTime<Utc>,
    linked_checklist_plan: ImplementationPlan,
) {
    let mut app = create_test_app(vec![
        create_test_session_at("selected", timestamp),
        create_test_session_at("other", timestamp),
    ]);
    app.update_row_order(&[None, Some("selected"), Some("other")]);
    app.list_state.select(Some(1));
    app.begin_progress_refresh("selected");
    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            delegated: Vec::new(),
            implementation: Some(linked_checklist_plan),
            github_error: None,
        }),
    );

    app.list_state.select(Some(2));
    app.begin_progress_refresh("other");
    app.list_state.select(Some(1));
    app.begin_progress_refresh("selected");

    assert_eq!(
        app.progress_rows,
        expected_unavailable_checklist_rows("selected"),
    );
}

#[rstest]
fn resets_checklist_folds_when_selection_changes_during_refresh(
    timestamp: DateTime<Utc>,
    linked_checklist_plan: ImplementationPlan,
) {
    let mut app = create_test_app(vec![
        create_test_session_at("selected", timestamp),
        create_test_session_at("other", timestamp),
    ]);
    app.update_row_order(&[None, Some("selected"), Some("other")]);
    app.list_state.select(Some(1));
    app.begin_progress_refresh("selected");
    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            delegated: Vec::new(),
            implementation: Some(linked_checklist_plan.clone()),
            github_error: None,
        }),
    );
    app.set_progress_batch_collapsed(true);

    app.list_state.select(Some(2));
    app.refresh_progress_from_sessions("other");
    app.begin_progress_refresh("other");
    app.apply_progress_result(
        "other",
        Ok(FetchedProgress {
            selected_session_id: "other".to_string(),
            delegated: Vec::new(),
            implementation: Some(linked_checklist_plan),
            github_error: None,
        }),
    );

    assert_eq!(
        app.progress_rows,
        expected_unavailable_checklist_rows("other"),
    );
}

#[rstest]
fn keeps_linked_pr_lookup_error_when_checklist_has_no_delegates(
    timestamp: DateTime<Utc>,
    linked_checklist_plan: ImplementationPlan,
) {
    let mut app = create_test_app(vec![create_test_session_at("selected", timestamp)]);
    app.begin_progress_refresh("selected");
    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            delegated: Vec::new(),
            implementation: Some(linked_checklist_plan),
            github_error: Some("GitHub unavailable".to_string()),
        }),
    );

    assert_eq!(
        (app.progress_rows, app.progress_error),
        (
            expected_unavailable_checklist_rows("selected"),
            Some("GitHub unavailable".to_string()),
        ),
    );
}

#[rstest]
fn preserves_fetch_error_when_no_delegate_rows_exist(timestamp: DateTime<Utc>) {
    let mut app = create_test_app(vec![create_test_session_at("selected", timestamp)]);
    app.begin_progress_refresh("selected");

    app.apply_progress_result("selected", Err("progress fetch failed".to_string()));

    assert_eq!(
        (
            app.progress_rows,
            app.progress_error,
            app.progress_session_id,
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        ),
        (
            vec![ProgressRow::Work(WorkProgressRow {
                session_id: "selected".to_string(),
                role: "session".to_string(),
                mark: ProgressMark::Working,
                status_label: None,
                note: None,
                pane_id: None,
            })],
            Some("progress fetch failed".to_string()),
            Some("selected".to_string()),
            Some(0),
            None,
        ),
    );
}

#[rstest]
fn falls_back_to_delegate_rows_when_checklist_progress_is_unavailable(
    timestamp: DateTime<Utc>,
    delegated_session: Session,
) {
    let selected = create_test_session_at("selected", timestamp);
    let fetched_delegate = FetchedDelegate {
        session: delegated_session.clone(),
        checklist_item_id: None,
        pull_request: None,
        lookup_unavailable: false,
    };
    let expected = build_progress_rows(
        &selected,
        std::slice::from_ref(&fetched_delegate),
        &AgentConfig::default(),
    );
    let mut app = create_test_app(vec![selected, delegated_session]);
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
