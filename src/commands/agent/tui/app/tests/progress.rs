use std::collections::HashMap;

use super::super::progress::CachedProgress;
use super::super::{App, ProgressScopeKind};
use super::{create_test_app, create_test_session_at};
use crate::commands::agent::tui::progress::{
    FetchedDelegate, FetchedProgress, PrProgressRow, ProgressFetchScope, ProgressFetchState,
    ProgressMark, ProgressRow, ProgressSelectionKey, StageMarks, WorkProgressRow,
    build_progress_rows,
};
use crate::commands::agent::tui::progress_batches::{
    ImplementationPlan, build_implementation_plan,
};
use crate::commands::agent::tui::session_rows::{SessionRowKey, SessionTask};
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
    session.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
    session.ancestor_session_ids = vec!["selected".to_string()];
    session
}

#[fixture]
fn selected_task_session(timestamp: DateTime<Utc>) -> Session {
    create_test_session_at("selected", timestamp)
}

#[fixture]
fn inline_progress_app(timestamp: DateTime<Utc>) -> App {
    let root = create_test_session_at("design", timestamp);
    let mut worker = create_test_session_at("worker", timestamp);
    worker.ancestor_session_ids = vec![root.session_id.clone()];
    let mut worker_child = create_test_session_at("worker-child", timestamp);
    worker_child.ancestor_session_ids = vec![root.session_id.clone(), worker.session_id.clone()];
    let mut unmatched = create_test_session_at("unmatched", timestamp);
    unmatched.ancestor_session_ids = vec![root.session_id.clone()];
    let mut app = create_test_app(vec![root, worker, worker_child, unmatched]);
    app.progress_inline_expanded = true;
    app.progress_session_id = Some("design".to_string());
    app.progress_fetch_states
        .insert("design".to_string(), ProgressFetchState::Ready);
    app.progress_rows = vec![
        ProgressRow::ImplementationHeader {
            merged_pr_count: 0,
            total_pr_count: 2,
            collapsed: false,
        },
        ProgressRow::BatchHeader {
            batch_id: "batch-a".to_string(),
            title: "Batch A".to_string(),
            merged_pr_count: 0,
            collapsed: false,
        },
        ProgressRow::ChecklistPullRequest {
            batch_id: "batch-a".to_string(),
            checklist_item_id: "item-a".to_string(),
            row: PrProgressRow {
                session_id: "worker".to_string(),
                repo: None,
                title: "Worker PR".to_string(),
                number: Some(23),
                url: Some("https://example.test/pull/23".to_string()),
                pane_id: None,
                is_ended: false,
                lookup_unavailable: false,
                stages: StageMarks {
                    implementation: ProgressMark::Done,
                    diff_review: ProgressMark::Done,
                    pr_body: ProgressMark::Done,
                    ci: ProgressMark::Waiting,
                    merge: ProgressMark::NotStarted,
                },
            },
        },
        ProgressRow::PlannedPullRequest {
            batch_id: "batch-a".to_string(),
            checklist_item_id: "item-b".to_string(),
            title: "Planned PR".to_string(),
        },
    ];
    app.rebuild_row_order();
    app
}

#[rstest]
fn inline_plan_merges_its_worker_once_and_keeps_unmatched_session_children(
    inline_progress_app: App,
) {
    assert_eq!(
        inline_progress_app.row_keys,
        vec![
            None,
            Some(SessionRowKey::Task("design".to_string())),
            Some(SessionRowKey::Progress {
                source_session_id: "design".to_string(),
                selection_key: ProgressSelectionKey::Implementation,
                session_id: None,
            }),
            Some(SessionRowKey::Progress {
                source_session_id: "design".to_string(),
                selection_key: ProgressSelectionKey::Batch("batch-a".to_string()),
                session_id: None,
            }),
            Some(SessionRowKey::Progress {
                source_session_id: "design".to_string(),
                selection_key: ProgressSelectionKey::ChecklistItem {
                    batch_id: "batch-a".to_string(),
                    item_id: "item-a".to_string(),
                },
                session_id: Some("worker".to_string()),
            }),
            Some(SessionRowKey::Task("worker-child".to_string())),
            Some(SessionRowKey::Progress {
                source_session_id: "design".to_string(),
                selection_key: ProgressSelectionKey::ChecklistItem {
                    batch_id: "batch-a".to_string(),
                    item_id: "item-b".to_string(),
                },
                session_id: None,
            }),
            Some(SessionRowKey::Task("unmatched".to_string())),
        ],
    );
}

#[rstest]
fn collapsing_a_checklist_item_selects_its_batch_header(
    timestamp: DateTime<Utc>,
    inline_progress_app: App,
    linked_checklist_plan: ImplementationPlan,
) {
    let mut app = inline_progress_app;
    app.set_cached_progress(
        "design",
        CachedProgress {
            rows: app.progress_rows.clone(),
            error: None,
            implementation: Some(linked_checklist_plan),
            request_scope: None,
            delegated_sessions: Vec::new(),
            fetched_at: Some(timestamp),
            last_attempt_at: Some(timestamp),
        },
    );
    let item_key = SessionRowKey::Progress {
        source_session_id: "design".to_string(),
        selection_key: ProgressSelectionKey::ChecklistItem {
            batch_id: "batch-a".to_string(),
            item_id: "item-a".to_string(),
        },
        session_id: Some("worker".to_string()),
    };
    let position = app
        .row_keys
        .iter()
        .position(|key| key.as_ref() == Some(&item_key))
        .expect("checklist row should be visible");
    app.list_state.select(Some(position));
    app.progress_list_state.select(Some(2));

    let changed = app.set_inline_progress_heading_expanded(false);

    assert_eq!(
        (
            changed,
            app.progress_batch_expanded.get("batch-a").copied(),
            app.selected_row_key().cloned(),
        ),
        (
            true,
            Some(false),
            Some(SessionRowKey::Progress {
                source_session_id: "design".to_string(),
                selection_key: ProgressSelectionKey::Batch("batch-a".to_string()),
                session_id: None,
            }),
        ),
    );
}

#[rstest]
#[case::not_started(None, "PR progress not fetched")]
#[case::loading(Some(ProgressFetchState::Loading), "Refreshing PR progress…")]
#[case::no_checklist(Some(ProgressFetchState::Ready), "No implementation checklist")]
#[case::failed(
    Some(ProgressFetchState::Failed("network unavailable".to_string())),
    "PR progress fetch failed: network unavailable"
)]
fn inline_status_distinguishes_fetch_state_from_missing_checklist(
    timestamp: DateTime<Utc>,
    #[case] fetch_state: Option<ProgressFetchState>,
    #[case] expected: &str,
) {
    let mut app = create_test_app(vec![create_test_session_at("design", timestamp)]);
    app.progress_inline_expanded = true;
    app.progress_session_id = Some("design".to_string());
    if let Some(state) = fetch_state {
        app.progress_fetch_states
            .insert("design".to_string(), state);
    }

    let rows = app.build_display_rows();
    let status_rows = rows
        .iter()
        .filter_map(|row| match row {
            crate::commands::agent::tui::session_rows::SessionRow::InlineProgress(entry) => {
                entry.label.clone()
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(status_rows, vec![expected.to_string()]);
}

#[rstest]
fn inline_status_displays_cached_pr_lookup_error(timestamp: DateTime<Utc>) {
    let mut app = create_test_app(vec![create_test_session_at("source-session", timestamp)]);
    app.progress_inline_expanded = true;
    app.progress_session_id = Some("source-session".to_string());
    app.progress_fetch_states
        .insert("source-session".to_string(), ProgressFetchState::Ready);
    app.set_cached_progress(
        "source-session",
        CachedProgress {
            rows: Vec::new(),
            error: Some("GitHub unavailable".to_string()),
            implementation: None,
            request_scope: None,
            delegated_sessions: Vec::new(),
            fetched_at: Some(timestamp),
            last_attempt_at: Some(timestamp),
        },
    );

    let status_rows = app
        .build_display_rows()
        .iter()
        .filter_map(|row| match row {
            crate::commands::agent::tui::session_rows::SessionRow::InlineProgress(entry) => {
                entry.label.clone()
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        status_rows,
        vec!["PR status lookup failed: GitHub unavailable".to_string()],
    );
}

fn progress_delegate(id: &str, timestamp: DateTime<Utc>) -> Session {
    let mut session = create_test_session_at(id, timestamp);
    session.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
    session.ancestor_session_ids = vec!["selected".to_string()];
    session
}

fn fetched_progress_delegate(session: Session) -> FetchedDelegate {
    FetchedDelegate {
        pull_request: Some(PrProgressInfo {
            pull_request: PrInfo {
                number: 42,
                title: format!("{} change", session.session_id),
                state: PrState::Open,
                url: format!("https://example.test/pull/{}", session.session_id),
            },
            ci_state: PrCiState::InProgress,
        }),
        session,
        checklist_item_id: None,
        lookup_unavailable: false,
    }
}

fn unmatched_progress_plan(delegates: &[FetchedDelegate]) -> ImplementationPlan {
    build_implementation_plan(
        &TqChecklist {
            id: "checklist-test".to_string(),
            name: "Implementation".to_string(),
            items: Vec::new(),
        },
        &[],
        delegates,
        &HashMap::new(),
        false,
    )
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
        ProgressRow::BatchHeader {
            batch_id: "batch-a".to_string(),
            title: "Batch 1".to_string(),
            merged_pr_count: 0,
            collapsed: false,
        },
        ProgressRow::ChecklistPullRequest {
            batch_id: "batch-a".to_string(),
            checklist_item_id: "item-a".to_string(),
            row: PrProgressRow {
                session_id: "checklist-item:item-a".to_string(),
                repo: Some("example-project".to_string()),
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
        repo: None,
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
            request_scope: None,
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
    delegate.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
    delegate.ancestor_session_ids = vec!["selected".to_string(), "same-task-child".to_string()];
    let mut nested_delegate = create_test_session_at("nested-delegate", timestamp);
    nested_delegate.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
    nested_delegate.ancestor_session_ids = vec![
        "selected".to_string(),
        "same-task-child".to_string(),
        "delegate".to_string(),
    ];
    let mut other_task_delegate = create_test_session_at("other-task-delegate", timestamp);
    other_task_delegate.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
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
        ("delegate".to_string(), vec![task_link("task-a")]),
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
            request_scope: None,
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
                expected_work_row("delegate", "session"),
                expected_work_row("unrelated-same-task", "session"),
                expected_unlinked_delegate_row("delegate"),
                expected_unlinked_delegate_row("nested-delegate"),
            ],
            Some(1),
            Some(ProgressSelectionKey::Session("selected".to_string())),
            None,
        ),
    );
}

#[rstest]
fn task_linked_delegate_stays_visible_when_its_parent_is_absent(timestamp: DateTime<Utc>) {
    let selected = create_test_session_at("selected", timestamp);
    let mut delegate = create_test_session_at("orphan-delegate", timestamp);
    delegate.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
    delegate.ancestor_session_ids = vec!["closed-parent".to_string()];
    let mut app = create_test_app(vec![selected, delegate]);
    app.update_row_order(&[None, Some("selected"), Some("orphan-delegate")]);
    app.list_state.select(Some(1));
    app.set_tq_snapshot(TqSnapshot::new(HashMap::from([
        ("selected".to_string(), vec![task_link("task-a")]),
        ("orphan-delegate".to_string(), vec![task_link("task-a")]),
    ])));

    app.begin_progress_refresh("selected");

    assert_eq!(
        app.progress_rows,
        vec![
            expected_work_row("selected", "session"),
            expected_work_row("orphan-delegate", "session"),
        ],
    );
}

#[rstest]
fn task_linked_delegate_without_work_rows_stays_in_the_checklist(timestamp: DateTime<Utc>) {
    let mut delegate = create_test_session_at("orphan-delegate", timestamp);
    delegate.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
    delegate.ancestor_session_ids = vec!["closed-parent".to_string()];
    let mut app = create_test_app(vec![delegate.clone()]);
    app.update_row_order(&[None, Some("orphan-delegate")]);
    app.list_state.select(Some(1));
    app.set_tq_snapshot(TqSnapshot::new(HashMap::from([(
        "orphan-delegate".to_string(),
        vec![task_link("task-a")],
    )])));
    let fetched_delegate = FetchedDelegate {
        session: delegate,
        checklist_item_id: None,
        pull_request: None,
        lookup_unavailable: false,
    };
    let plan = build_implementation_plan(
        &TqChecklist {
            id: "checklist-a".to_string(),
            name: "実装".to_string(),
            items: Vec::new(),
        },
        &[],
        std::slice::from_ref(&fetched_delegate),
        &HashMap::new(),
        false,
    );

    app.apply_progress_result(
        "orphan-delegate",
        Ok(FetchedProgress {
            selected_session_id: "orphan-delegate".to_string(),
            request_scope: None,
            delegated: vec![fetched_delegate],
            implementation: Some(plan),
            github_error: None,
        }),
    );

    assert_eq!(
        app.progress_rows,
        vec![
            expected_work_row("orphan-delegate", "session"),
            ProgressRow::ImplementationHeader {
                merged_pr_count: 0,
                total_pr_count: 0,
                collapsed: false,
            },
            ProgressRow::UnmatchedDelegatesHeader { collapsed: false },
            expected_unlinked_delegate_row("orphan-delegate"),
        ],
    );
}

#[rstest]
fn task_progress_applies_fetched_delegates_missing_from_the_live_scope(timestamp: DateTime<Utc>) {
    let selected = create_test_session_at("selected", timestamp);
    let mut live_delegate = create_test_session_at("live-delegate", timestamp);
    live_delegate.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
    live_delegate.ancestor_session_ids = vec!["selected".to_string()];
    let mut fetched_delegate = create_test_session_at("fetched-delegate", timestamp);
    fetched_delegate.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
    fetched_delegate.ancestor_session_ids = vec!["selected".to_string()];
    let mut app = create_test_app(vec![selected, live_delegate.clone()]);
    app.update_row_order(&[None, Some("selected"), Some("live-delegate")]);
    app.list_state.select(Some(1));
    app.set_tq_snapshot(TqSnapshot::new(HashMap::from([(
        "selected".to_string(),
        vec![task_link("task-a")],
    )])));
    let fetched_delegates = vec![
        FetchedDelegate {
            session: live_delegate,
            checklist_item_id: None,
            pull_request: None,
            lookup_unavailable: false,
        },
        FetchedDelegate {
            session: fetched_delegate,
            checklist_item_id: None,
            pull_request: None,
            lookup_unavailable: false,
        },
    ];
    let plan = build_implementation_plan(
        &TqChecklist {
            id: "checklist-a".to_string(),
            name: "実装".to_string(),
            items: Vec::new(),
        },
        &[],
        &fetched_delegates,
        &HashMap::new(),
        false,
    );

    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            request_scope: Some(ProgressFetchScope {
                task_id: Some("task-a".to_string()),
                session_ids: Some(vec!["selected".to_string()]),
                other_task_session_ids: vec![],
            }),
            delegated: fetched_delegates,
            implementation: Some(plan),
            github_error: None,
        }),
    );

    assert_eq!(
        app.progress_rows,
        vec![
            expected_work_row("selected", "session"),
            ProgressRow::ImplementationHeader {
                merged_pr_count: 0,
                total_pr_count: 0,
                collapsed: false,
            },
            ProgressRow::UnmatchedDelegatesHeader { collapsed: false },
            expected_unlinked_delegate_row("live-delegate"),
            expected_unlinked_delegate_row("fetched-delegate"),
        ],
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
            request_scope: None,
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
                expected_unavailable_checklist_rows("selected")[3].clone(),
            ],
            Some(0),
            None,
        ),
    );
}

#[rstest]
#[case::task_linked(true)]
#[case::unlinked(false)]
fn project_manager_progress_shows_only_its_work_row_regardless_of_task_link(
    #[case] task_linked: bool,
    timestamp: DateTime<Utc>,
    linked_checklist_plan: ImplementationPlan,
) {
    let mut project_manager = create_test_session_at("project-manager", timestamp);
    project_manager.work_type = Some("project-manager".to_string());
    let mut child_task = create_test_session_at("child-task", timestamp);
    child_task.ancestor_session_ids = vec!["project-manager".to_string()];
    let mut child_delegate = create_test_session_at("child-delegate", timestamp);
    child_delegate.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
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
    if task_linked {
        app.set_tq_snapshot(TqSnapshot::new(HashMap::from([
            (
                "project-manager".to_string(),
                vec![task_link("parent-task")],
            ),
            ("child-task".to_string(), vec![task_link("parent-task")]),
        ])));
    }

    let fetch_scope = app.progress_scope_for_session("project-manager");

    app.apply_progress_result(
        "project-manager",
        Ok(FetchedProgress {
            selected_session_id: "project-manager".to_string(),
            request_scope: None,
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
    let fetched_progress = (
        app.progress_rows.clone(),
        app.progress_list_state.selected(),
        app.progress_table_state.selected(),
    );

    app.refresh_progress_from_sessions("project-manager");

    assert_eq!(
        (
            fetch_scope,
            fetched_progress,
            (
                app.progress_rows.clone(),
                app.progress_list_state.selected(),
                app.progress_table_state.selected(),
            ),
        ),
        (Some(ProgressScopeKind::OwnOnly), expected.clone(), expected,),
    );
}

#[rstest]
fn local_progress_refresh_uses_current_delegate_state(
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
            request_scope: None,
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
                    repo: None,
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
            Some(1),
            Some(0),
        ),
    );
}

#[rstest]
fn initial_focus_keeps_the_worker_selected_and_opens_parent_progress(
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
            app.progress_inline_expanded,
            app.selected_progress_source_id(),
        ),
        (Some("delegate"), true, Some("selected".to_string())),
    );
}

#[rstest]
#[case::same_task_keeps_the_parent_source("source-task", "source-session")]
#[case::different_task_uses_the_selected_source("other-task", "selected-session")]
fn selected_linked_session_uses_the_matching_task_source(
    timestamp: DateTime<Utc>,
    #[case] selected_task_id: &str,
    #[case] expected_source_id: &str,
) {
    let source = create_test_session_at("source-session", timestamp);
    let mut selected = create_test_session_at("selected-session", timestamp);
    selected.ancestor_session_ids = vec![source.session_id.clone()];
    let mut app = create_test_app(vec![source, selected]);
    app.task_by_session
        .insert("source-session".to_string(), task_link("source-task"));
    app.task_by_session
        .insert("selected-session".to_string(), task_link(selected_task_id));
    app.progress_session_id = Some("source-session".to_string());
    app.update_row_order(&[None, Some("source-session"), Some("selected-session")]);
    let selected_position = app
        .row_keys
        .iter()
        .position(|key| key.as_ref() == Some(&SessionRowKey::Task("selected-session".to_string())))
        .expect("selected session should be visible");
    app.list_state.select(Some(selected_position));

    assert_eq!(
        app.selected_progress_source_id(),
        Some(expected_source_id.to_string()),
    );
}

#[rstest]
fn nested_delegate_focus_keeps_the_worker_selected_in_its_parent_tree(timestamp: DateTime<Utc>) {
    let root = create_test_session_at("root", timestamp);
    let mut task = create_test_session_at("task", timestamp);
    task.ancestor_session_ids = vec![root.session_id.clone()];
    let mut intermediate_delegate = create_test_session_at("intermediate-delegate", timestamp);
    intermediate_delegate.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
    intermediate_delegate.ancestor_session_ids =
        vec![root.session_id.clone(), task.session_id.clone()];
    let mut delegate = create_test_session_at("delegate", timestamp);
    delegate.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
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

    app.begin_progress_refresh("intermediate-delegate");

    assert_eq!(
        (
            app.selected_session_id(),
            app.progress_inline_expanded,
            app.selected_progress_source_id(),
        ),
        (
            Some("delegate"),
            true,
            Some("intermediate-delegate".to_string()),
        ),
    );
}

#[rstest]
fn initial_focus_expands_the_worker_batch_without_moving_the_cursor(
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
            request_scope: None,
            delegated: fetched_delegates,
            implementation: Some(implementation),
            github_error: None,
        }),
    );

    assert_eq!(
        (
            app.selected_session_id(),
            app.progress_inline_expanded,
            app.progress_implementation_expanded,
            app.progress_batch_expanded.get("batch-a").copied(),
            app.list_state
                .selected()
                .and_then(|position| app.row_keys.get(position))
                .and_then(Option::as_ref)
                .cloned(),
        ),
        (
            Some("delegate"),
            true,
            true,
            Some(true),
            Some(
                crate::commands::agent::tui::session_rows::SessionRowKey::Progress {
                    source_session_id: "selected".to_string(),
                    selection_key: ProgressSelectionKey::ChecklistItem {
                        batch_id: "batch-a".to_string(),
                        item_id: "item-a".to_string(),
                    },
                    session_id: Some("delegate".to_string()),
                }
            ),
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
                repo: None,
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
            request_scope: None,
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
                    repo: None,
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
            request_scope: None,
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
                    repo: None,
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
fn keeps_fetched_checklist_rows_while_a_session_reload_is_incomplete(
    timestamp: DateTime<Utc>,
    delegated_session: Session,
) {
    let selected = create_test_session_at("selected", timestamp);
    let fetched_delegate = FetchedDelegate {
        session: delegated_session.clone(),
        checklist_item_id: None,
        pull_request: None,
        lookup_unavailable: true,
    };
    let plan = build_implementation_plan(
        &TqChecklist {
            id: "checklist-a".to_string(),
            name: "実装".to_string(),
            items: Vec::new(),
        },
        &[],
        std::slice::from_ref(&fetched_delegate),
        &HashMap::new(),
        false,
    );
    let mut app = create_test_app(vec![selected, delegated_session]);
    app.update_row_order(&[None, Some("selected"), Some("delegate")]);
    app.list_state.select(Some(1));
    app.begin_progress_refresh("selected");
    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            request_scope: None,
            delegated: vec![fetched_delegate],
            implementation: Some(plan),
            github_error: None,
        }),
    );
    let before = (
        app.progress_rows.clone(),
        app.progress_list_state.selected(),
        app.progress_table_state.selected(),
    );

    app.sessions
        .retain(|session| session.session_id == "selected");
    app.refresh_progress_from_sessions("selected");

    assert_eq!(
        (
            app.progress_rows,
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        ),
        before,
    );
}

#[rstest]
fn reopening_unmatched_delegates_keeps_cached_rows_after_session_reload_is_incomplete(
    timestamp: DateTime<Utc>,
) {
    let selected = create_test_session_at("selected", timestamp);
    let alpha = progress_delegate("alpha", timestamp);
    let bravo = progress_delegate("bravo", timestamp);
    let charlie = progress_delegate("charlie", timestamp);
    let fetched = vec![
        fetched_progress_delegate(alpha.clone()),
        fetched_progress_delegate(bravo.clone()),
        fetched_progress_delegate(charlie.clone()),
    ];
    let plan = unmatched_progress_plan(&fetched);
    let mut app = create_test_app(vec![selected, alpha, bravo, charlie]);
    app.update_row_order(&[
        None,
        Some("selected"),
        Some("alpha"),
        Some("bravo"),
        Some("charlie"),
    ]);
    app.list_state.select(Some(1));
    app.begin_progress_refresh("selected");
    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            request_scope: None,
            delegated: fetched,
            implementation: Some(plan),
            github_error: None,
        }),
    );
    app.sessions
        .retain(|session| session.session_id == "selected" || session.session_id == "alpha");
    app.refresh_progress_from_sessions("selected");
    let header_index = app
        .progress_rows
        .iter()
        .position(|row| matches!(row, ProgressRow::UnmatchedDelegatesHeader { .. }))
        .unwrap();
    app.progress_list_state.select(Some(header_index));

    app.set_progress_batch_collapsed(true);
    app.toggle_selected_progress_heading();

    let delegate_ids = app
        .progress_rows
        .iter()
        .filter_map(|row| match row {
            ProgressRow::PullRequest(row) => Some(row.session_id.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(delegate_ids, vec!["alpha", "bravo", "charlie"]);
}

#[rstest]
fn progress_refresh_keeps_cached_delegate_order_and_uses_current_session_state(
    timestamp: DateTime<Utc>,
) {
    let selected = create_test_session_at("selected", timestamp);
    let alpha = progress_delegate("alpha", timestamp);
    let bravo = progress_delegate("bravo", timestamp);
    let charlie = progress_delegate("charlie", timestamp);
    let delta = progress_delegate("delta", timestamp);
    let initial = vec![
        fetched_progress_delegate(alpha.clone()),
        fetched_progress_delegate(bravo.clone()),
        fetched_progress_delegate(charlie.clone()),
    ];
    let initial_plan = unmatched_progress_plan(&initial);
    let mut app = create_test_app(vec![
        selected,
        alpha.clone(),
        bravo.clone(),
        charlie.clone(),
        delta.clone(),
    ]);
    app.update_row_order(&[
        None,
        Some("selected"),
        Some("alpha"),
        Some("bravo"),
        Some("charlie"),
        Some("delta"),
    ]);
    app.list_state.select(Some(1));
    app.begin_progress_refresh("selected");
    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            request_scope: None,
            delegated: initial,
            implementation: Some(initial_plan),
            github_error: None,
        }),
    );
    app.sessions
        .iter_mut()
        .find(|session| session.session_id == "alpha")
        .unwrap()
        .status = SessionStatus::Ended;
    let refreshed = vec![
        fetched_progress_delegate(charlie),
        fetched_progress_delegate(alpha),
        fetched_progress_delegate(bravo),
        fetched_progress_delegate(delta),
    ];
    let refreshed_plan = unmatched_progress_plan(&refreshed);

    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            request_scope: None,
            delegated: refreshed,
            implementation: Some(refreshed_plan),
            github_error: None,
        }),
    );

    let rows = app
        .progress_rows
        .iter()
        .filter_map(|row| match row {
            ProgressRow::PullRequest(row) => Some((row.session_id.as_str(), row.is_ended)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rows,
        vec![
            ("alpha", true),
            ("bravo", false),
            ("charlie", false),
            ("delta", false),
        ],
    );
}

#[rstest]
#[case::task_id_changed("task-b", false, vec!["delegate", "selected"])]
#[case::task_membership_changed("task-a", true, vec!["delegate", "selected"])]
fn ignores_progress_results_from_an_outdated_task_scope(
    timestamp: DateTime<Utc>,
    delegated_session: Session,
    #[case] stale_task_id: &str,
    #[case] add_linked_session: bool,
    #[case] stale_session_ids: Vec<&str>,
) {
    let selected = create_test_session_at("selected", timestamp);
    let linked_later = create_test_session_at("linked-later", timestamp);
    let mut app = create_test_app(vec![selected, delegated_session, linked_later]);
    app.update_row_order(&[
        None,
        Some("selected"),
        Some("delegate"),
        Some("linked-later"),
    ]);
    app.list_state.select(Some(1));
    app.set_tq_snapshot(TqSnapshot::new(HashMap::from([
        ("selected".to_string(), vec![task_link("task-a")]),
        ("delegate".to_string(), vec![task_link("task-a")]),
    ])));
    app.begin_progress_refresh("selected");
    if add_linked_session {
        app.set_tq_snapshot(TqSnapshot::new(HashMap::from([
            ("selected".to_string(), vec![task_link("task-a")]),
            ("delegate".to_string(), vec![task_link("task-a")]),
            ("linked-later".to_string(), vec![task_link("task-a")]),
        ])));
        app.refresh_progress_from_sessions("selected");
    }
    let before = app.progress_rows.clone();

    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            request_scope: Some(ProgressFetchScope {
                task_id: Some(stale_task_id.to_string()),
                session_ids: Some(
                    stale_session_ids
                        .iter()
                        .map(|session_id| session_id.to_string())
                        .collect(),
                ),
                other_task_session_ids: vec![],
            }),
            delegated: Vec::new(),
            implementation: None,
            github_error: None,
        }),
    );

    assert_eq!(app.progress_rows, before);
}

#[rstest]
fn omits_ended_delegates_without_pull_request_info(
    timestamp: DateTime<Utc>,
    mut delegated_session: Session,
) {
    delegated_session.status = SessionStatus::Ended;
    let selected = create_test_session_at("selected", timestamp);
    let mut app = create_test_app(vec![selected, delegated_session.clone()]);
    app.update_row_order(&[None, Some("selected"), Some("delegate")]);
    app.list_state.select(Some(1));
    app.begin_progress_refresh("selected");
    app.apply_progress_result(
        "selected",
        Ok(FetchedProgress {
            selected_session_id: "selected".to_string(),
            request_scope: None,
            delegated: vec![FetchedDelegate {
                session: delegated_session,
                checklist_item_id: None,
                pull_request: None,
                lookup_unavailable: true,
            }],
            implementation: None,
            github_error: None,
        }),
    );

    assert_eq!(
        app.progress_rows,
        vec![expected_work_row("selected", "session")]
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
                    repo: None,
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
            request_scope: None,
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
            request_scope: None,
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
            request_scope: None,
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
            request_scope: None,
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
            request_scope: None,
            delegated: vec![fetched_delegate],
            implementation: None,
            github_error: None,
        }),
    );

    assert_eq!(app.progress_rows, expected);
}
