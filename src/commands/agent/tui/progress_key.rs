use crossterm::event::{KeyCode, KeyModifiers};

use super::KeyEffects;
use super::app::App;
use super::event::KeyEvent;
use super::progress::{open_pr_url_target, open_target};

pub(super) fn handle(app: &mut App, key: KeyEvent) -> Option<KeyEffects> {
    if !app.progress_focused {
        return None;
    }

    app.clear_error();
    let current_rows = app.progress_session_id.as_deref() == app.selected_session_id();
    match (key.code, key.modifiers) {
        (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) if current_rows => {
            app.select_progress_row(1);
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) if current_rows => {
            app.select_progress_row(-1);
        }
        (KeyCode::Left, _) => {
            app.progress_focused = false;
        }
        (KeyCode::Char('h'), KeyModifiers::NONE) if current_rows => {
            app.set_progress_batch_collapsed(true);
        }
        (KeyCode::Char('l'), KeyModifiers::NONE) if current_rows => {
            app.set_progress_batch_collapsed(false);
        }
        (KeyCode::Enter, _) if current_rows => {
            if !app.toggle_selected_progress_heading() {
                let target = app
                    .progress_list_state
                    .selected()
                    .and_then(|index| app.progress_rows.get(index))
                    .filter(|row| {
                        row.session_id().is_some_and(|session_id| {
                            app.sessions
                                .iter()
                                .any(|session| session.session_id == session_id)
                        })
                    })
                    .and_then(open_target);
                return Some(KeyEffects {
                    open_progress_target: target,
                    ..Default::default()
                });
            }
        }
        (KeyCode::Char('o'), KeyModifiers::NONE) if current_rows => {
            return Some(KeyEffects {
                open_progress_target: app
                    .progress_list_state
                    .selected()
                    .and_then(|index| app.progress_rows.get(index))
                    .and_then(open_pr_url_target),
                ..Default::default()
            });
        }
        (KeyCode::Char('q'), KeyModifiers::NONE) => app.quit(),
        (KeyCode::Char('?'), KeyModifiers::NONE) => app.toggle_help(),
        (KeyCode::Esc, _) => {
            app.progress_focused = false;
        }
        _ => {}
    }
    Some(KeyEffects::default())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use chrono::Utc;
    use crossterm::event::KeyCode;
    use rstest::{fixture, rstest};

    use super::*;
    use crate::commands::agent::tui::progress::{
        FetchedDelegate, FetchedProgress, PrProgressRow, ProgressMark, ProgressOpenTarget,
        ProgressRow, ProgressSelectionKey, StageMarks, WorkProgressRow,
    };
    use crate::commands::agent::tui::progress_batches::build_implementation_plan;
    use crate::commands::agent::types::{Engine, Session, SessionStatus};
    use crate::infra::tq::{TqChecklist, TqChecklistItem};

    fn session(session_id: &str) -> Session {
        let now = Utc::now();
        Session {
            archived_at: None,
            pr_context: None,
            session_id: session_id.to_string(),
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/example-project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: now,
            updated_at: now,
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            launched_with_vibe: false,
            work_type_pinned: false,
            is_delegated: false,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: BTreeMap::new(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
        }
    }

    fn work_row() -> ProgressRow {
        ProgressRow::Work(WorkProgressRow {
            session_id: "selected".to_string(),
            role: "design".to_string(),
            mark: ProgressMark::Working,
            status_label: None,
            note: None,
            pane_id: None,
        })
    }

    fn pull_request_row(is_ended: bool, pane_id: Option<&str>, url: Option<&str>) -> ProgressRow {
        ProgressRow::PullRequest(PrProgressRow {
            repo: None,
            session_id: "delegate".to_string(),
            title: "Example delegated work".to_string(),
            number: Some(7),
            url: url.map(str::to_owned),
            pane_id: pane_id.map(str::to_owned),
            is_ended,
            lookup_unavailable: false,
            stages: StageMarks {
                implementation: ProgressMark::Done,
                diff_review: ProgressMark::User,
                pr_body: ProgressMark::NotStarted,
                ci: ProgressMark::NotStarted,
                merge: ProgressMark::NotStarted,
            },
        })
    }

    #[fixture]
    fn focused_progress_app() -> App {
        let mut app = App::with_sessions(vec![session("selected")]);
        app.progress_focused = true;
        app.progress_session_id = Some("selected".to_string());
        app
    }

    #[test]
    fn j_and_k_select_headings_and_skip_planned_rows() {
        let mut app = App::with_sessions(vec![session("selected")]);
        app.progress_focused = true;
        app.progress_session_id = Some("selected".to_string());
        app.progress_rows = vec![
            work_row(),
            ProgressRow::ImplementationHeader {
                merged_pr_count: 0,
                total_pr_count: 1,
                collapsed: false,
            },
            ProgressRow::BatchHeader {
                batch_id: "batch-1".to_string(),
                title: "Batch 1".to_string(),
                merged_pr_count: 0,
                collapsed: false,
            },
            pull_request_row(false, None, None),
            ProgressRow::PlannedPullRequest {
                batch_id: "batch-1".to_string(),
                checklist_item_id: "item-1".to_string(),
                title: "Future change".to_string(),
            },
        ];
        app.progress_list_state.select(Some(0));

        let mut actual = Vec::new();
        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('j'),
            KeyCode::Char('j'),
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Char('k'),
            KeyCode::Char('k'),
        ] {
            handle(
                &mut app,
                KeyEvent {
                    code,
                    modifiers: KeyModifiers::NONE,
                },
            );
            let index = app.progress_list_state.selected();
            actual.push((
                index.and_then(|index| app.progress_rows[index].selection_key()),
                index,
                app.progress_table_state.selected(),
            ));
        }

        assert_eq!(
            actual,
            vec![
                (Some(ProgressSelectionKey::Implementation), Some(1), Some(0),),
                (
                    Some(ProgressSelectionKey::Batch("batch-1".to_string())),
                    Some(2),
                    Some(1),
                ),
                (
                    Some(ProgressSelectionKey::Session("delegate".to_string())),
                    Some(3),
                    Some(2),
                ),
                (
                    Some(ProgressSelectionKey::Session("delegate".to_string())),
                    Some(3),
                    Some(2),
                ),
                (
                    Some(ProgressSelectionKey::Batch("batch-1".to_string())),
                    Some(2),
                    Some(1),
                ),
                (Some(ProgressSelectionKey::Implementation), Some(1), Some(0),),
                (
                    Some(ProgressSelectionKey::Session("selected".to_string())),
                    Some(0),
                    None,
                ),
            ],
        );
    }

    #[fixture]
    fn checklist_progress_app() -> App {
        let mut selected = session("selected");
        selected.work_type = Some("design".to_string());
        let mut delegate = session("delegate");
        delegate.is_delegated = true;
        delegate.ancestor_session_ids = vec!["selected".to_string()];
        let checklist = TqChecklist {
            id: "checklist-1".to_string(),
            name: "実装".to_string(),
            items: vec![TqChecklistItem {
                id: "batch-1".to_string(),
                content: "Batch 1".to_string(),
                checked_at: None,
                github_link_id: None,
                children: vec![
                    TqChecklistItem {
                        id: "item-1".to_string(),
                        content: "PR 1: Active change".to_string(),
                        checked_at: None,
                        github_link_id: None,
                        children: Vec::new(),
                    },
                    TqChecklistItem {
                        id: "item-2".to_string(),
                        content: "PR 2: Future change".to_string(),
                        checked_at: None,
                        github_link_id: None,
                        children: Vec::new(),
                    },
                ],
            }],
        };
        let fetched_delegates = vec![FetchedDelegate {
            session: delegate.clone(),
            checklist_item_id: Some("item-1".to_string()),
            pull_request: None,
            lookup_unavailable: false,
        }];
        let plan = build_implementation_plan(
            &checklist,
            &[],
            &fetched_delegates,
            &Default::default(),
            false,
        );
        let mut app = App::with_sessions(vec![selected.clone(), delegate.clone()]);
        app.progress_focused = true;
        app.update_row_order(&[None, Some("selected"), Some("delegate")]);
        app.list_state.select(Some(1));
        app.apply_progress_result(
            "selected",
            Ok(FetchedProgress {
                selected_session_id: "selected".to_string(),
                request_scope: None,
                delegated: fetched_delegates,
                implementation: Some(plan),
                github_error: None,
            }),
        );
        app.progress_list_state.select(Some(2));
        app.progress_table_state.select(Some(1));
        app
    }

    #[fixture]
    fn unmatched_progress_app() -> App {
        let mut selected = session("selected");
        selected.work_type = Some("design".to_string());
        let mut delegate = session("unmapped-delegate");
        delegate.is_delegated = true;
        delegate.ancestor_session_ids = vec!["selected".to_string()];
        delegate.label = Some("Unmapped delegated work".to_string());
        let fetched_delegate = FetchedDelegate {
            session: delegate.clone(),
            checklist_item_id: None,
            pull_request: None,
            lookup_unavailable: false,
        };
        let plan = build_implementation_plan(
            &TqChecklist {
                id: "checklist-1".to_string(),
                name: "実装".to_string(),
                items: Vec::new(),
            },
            &[],
            std::slice::from_ref(&fetched_delegate),
            &Default::default(),
            false,
        );
        let mut app = App::with_sessions(vec![selected, delegate]);
        app.progress_focused = true;
        app.update_row_order(&[None, Some("selected"), Some("unmapped-delegate")]);
        app.list_state.select(Some(1));
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
        app
    }

    fn send_progress_key(app: &mut App, code: KeyCode) {
        handle(
            app,
            KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
            },
        );
    }

    fn active_pr_row() -> ProgressRow {
        ProgressRow::ChecklistPullRequest {
            batch_id: "batch-1".to_string(),
            row: PrProgressRow {
                repo: None,
                session_id: "delegate".to_string(),
                title: "Active change".to_string(),
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
            },
        }
    }

    fn batch_header(collapsed: bool) -> ProgressRow {
        ProgressRow::BatchHeader {
            batch_id: "batch-1".to_string(),
            title: "Batch 1".to_string(),
            merged_pr_count: 0,
            collapsed,
        }
    }

    fn implementation_header(collapsed: bool) -> ProgressRow {
        ProgressRow::ImplementationHeader {
            merged_pr_count: 0,
            total_pr_count: 2,
            collapsed,
        }
    }

    fn collapsed_batch_rows() -> Vec<ProgressRow> {
        vec![work_row(), implementation_header(false), batch_header(true)]
    }

    fn expanded_batch_rows() -> Vec<ProgressRow> {
        vec![
            work_row(),
            implementation_header(false),
            batch_header(false),
            active_pr_row(),
            ProgressRow::PlannedPullRequest {
                batch_id: "batch-1".to_string(),
                checklist_item_id: "item-2".to_string(),
                title: "Future change".to_string(),
            },
        ]
    }

    fn collapsed_implementation_rows() -> Vec<ProgressRow> {
        vec![work_row(), implementation_header(true)]
    }

    fn expanded_implementation_rows() -> Vec<ProgressRow> {
        expanded_batch_rows()
    }

    fn unmatched_header(collapsed: bool) -> ProgressRow {
        ProgressRow::UnmatchedDelegatesHeader { collapsed }
    }

    fn unmatched_delegate_row() -> ProgressRow {
        ProgressRow::PullRequest(PrProgressRow {
            repo: None,
            session_id: "unmapped-delegate".to_string(),
            title: "Unmapped delegated work".to_string(),
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

    fn unmatched_rows(collapsed: bool) -> Vec<ProgressRow> {
        let mut rows = vec![
            work_row(),
            ProgressRow::ImplementationHeader {
                merged_pr_count: 0,
                total_pr_count: 0,
                collapsed: false,
            },
            unmatched_header(collapsed),
        ];
        if !collapsed {
            rows.push(unmatched_delegate_row());
        }
        rows
    }

    #[rstest]
    fn batch_heading_keys_fold_and_unfold_the_selected_batch(mut checklist_progress_app: App) {
        let mut actual = Vec::new();
        for code in [
            KeyCode::Char('h'),
            KeyCode::Char('l'),
            KeyCode::Enter,
            KeyCode::Enter,
        ] {
            send_progress_key(&mut checklist_progress_app, code);
            actual.push((
                checklist_progress_app.progress_rows.clone(),
                checklist_progress_app.progress_list_state.selected(),
                checklist_progress_app.progress_table_state.selected(),
            ));
        }

        assert_eq!(
            actual,
            vec![
                (collapsed_batch_rows(), Some(2), Some(1)),
                (expanded_batch_rows(), Some(2), Some(1)),
                (collapsed_batch_rows(), Some(2), Some(1)),
                (expanded_batch_rows(), Some(2), Some(1)),
            ],
        );
    }

    #[rstest]
    fn h_on_a_checklist_pr_collapses_its_batch_and_selects_the_heading(
        mut checklist_progress_app: App,
    ) {
        send_progress_key(&mut checklist_progress_app, KeyCode::Char('j'));
        send_progress_key(&mut checklist_progress_app, KeyCode::Char('h'));

        assert_eq!(
            (
                checklist_progress_app.progress_rows,
                checklist_progress_app.progress_list_state.selected(),
                checklist_progress_app.progress_table_state.selected(),
            ),
            (collapsed_batch_rows(), Some(2), Some(1)),
        );
    }

    #[rstest]
    fn implementation_heading_keys_fold_and_unfold_the_section(mut checklist_progress_app: App) {
        checklist_progress_app.progress_list_state.select(Some(1));
        checklist_progress_app.progress_table_state.select(Some(0));
        let expected_collapsed = collapsed_implementation_rows();
        let expected_expanded = expanded_implementation_rows();
        let mut actual = Vec::new();
        for code in [
            KeyCode::Char('h'),
            KeyCode::Char('l'),
            KeyCode::Enter,
            KeyCode::Enter,
        ] {
            send_progress_key(&mut checklist_progress_app, code);
            actual.push((
                checklist_progress_app.progress_rows.clone(),
                checklist_progress_app.progress_list_state.selected(),
                checklist_progress_app.progress_table_state.selected(),
            ));
        }

        assert_eq!(
            actual,
            vec![
                (expected_collapsed.clone(), Some(1), Some(0)),
                (expected_expanded.clone(), Some(1), Some(0)),
                (expected_collapsed, Some(1), Some(0)),
                (expected_expanded, Some(1), Some(0)),
            ],
        );
    }

    #[rstest]
    fn checklist_out_heading_is_selectable_and_toggles_its_rows(mut unmatched_progress_app: App) {
        unmatched_progress_app.progress_list_state.select(Some(1));
        unmatched_progress_app.progress_table_state.select(Some(0));
        send_progress_key(&mut unmatched_progress_app, KeyCode::Char('j'));
        let selected_heading = (
            unmatched_progress_app.progress_rows.clone(),
            unmatched_progress_app.progress_list_state.selected(),
            unmatched_progress_app
                .progress_rows
                .get(
                    unmatched_progress_app
                        .progress_list_state
                        .selected()
                        .unwrap_or(0),
                )
                .and_then(ProgressRow::selection_key),
        );
        let mut actual = vec![selected_heading];
        for code in [
            KeyCode::Char('h'),
            KeyCode::Char('l'),
            KeyCode::Enter,
            KeyCode::Enter,
        ] {
            send_progress_key(&mut unmatched_progress_app, code);
            actual.push((
                unmatched_progress_app.progress_rows.clone(),
                unmatched_progress_app.progress_list_state.selected(),
                unmatched_progress_app
                    .progress_rows
                    .get(
                        unmatched_progress_app
                            .progress_list_state
                            .selected()
                            .unwrap_or(0),
                    )
                    .and_then(ProgressRow::selection_key),
            ));
        }

        assert_eq!(
            actual,
            vec![
                (
                    unmatched_rows(false),
                    Some(2),
                    Some(ProgressSelectionKey::UnmatchedDelegates),
                ),
                (
                    unmatched_rows(true),
                    Some(2),
                    Some(ProgressSelectionKey::UnmatchedDelegates),
                ),
                (
                    unmatched_rows(false),
                    Some(2),
                    Some(ProgressSelectionKey::UnmatchedDelegates),
                ),
                (
                    unmatched_rows(true),
                    Some(2),
                    Some(ProgressSelectionKey::UnmatchedDelegates),
                ),
                (
                    unmatched_rows(false),
                    Some(2),
                    Some(ProgressSelectionKey::UnmatchedDelegates),
                ),
            ],
        );
    }

    #[rstest]
    fn h_on_a_checklist_out_pr_collapses_its_group_and_selects_the_heading(
        mut unmatched_progress_app: App,
    ) {
        unmatched_progress_app.progress_list_state.select(Some(3));
        unmatched_progress_app.progress_table_state.select(Some(2));
        send_progress_key(&mut unmatched_progress_app, KeyCode::Char('h'));

        assert_eq!(
            (
                unmatched_progress_app.progress_rows,
                unmatched_progress_app.progress_list_state.selected(),
                unmatched_progress_app.progress_table_state.selected(),
            ),
            (unmatched_rows(true), Some(2), Some(1)),
        );
    }

    #[rstest]
    #[case::focus_running_delegate(false, Some("%8"), Some("https://example.com/pull/7"), true, Some(ProgressOpenTarget::FocusPane("%8".to_string())))]
    #[case::ignore_ended_delegate(true, Some("%8"), Some("https://example.com/pull/7"), true, None)]
    #[case::ignore_missing_session(
        false,
        Some("%8"),
        Some("https://example.com/pull/7"),
        false,
        None
    )]
    #[case::active_pr_without_pane(false, None, Some("https://example.com/pull/7"), true, None)]
    #[case::ignore_row_without_target(false, None, None, true, None)]
    fn enter_opens_the_progress_row_target(
        #[case] is_ended: bool,
        #[case] pane_id: Option<&str>,
        #[case] url: Option<&str>,
        #[case] has_session: bool,
        #[case] expected_target: Option<ProgressOpenTarget>,
    ) {
        let mut sessions = vec![session("selected")];
        if has_session {
            let mut delegate = session("delegate");
            if is_ended {
                delegate.status = SessionStatus::Ended;
            }
            sessions.push(delegate);
        }
        let mut app = App::with_sessions(sessions);
        if app.selected_session_id() != Some("selected") {
            app.select_next();
        }
        app.progress_focused = true;
        app.progress_session_id = Some("selected".to_string());
        app.progress_rows = vec![work_row(), pull_request_row(is_ended, pane_id, url)];
        app.progress_list_state.select(Some(1));

        assert_eq!(
            handle(
                &mut app,
                KeyEvent {
                    code: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                },
            ),
            Some(KeyEffects {
                open_progress_target: expected_target,
                ..Default::default()
            }),
        );
    }

    #[rstest]
    fn o_opens_the_selected_pull_request_url(mut focused_progress_app: App) {
        focused_progress_app.progress_rows = vec![
            work_row(),
            pull_request_row(true, None, Some("https://example.com/pull/7")),
        ];
        focused_progress_app.progress_list_state.select(Some(1));

        assert_eq!(
            handle(
                &mut focused_progress_app,
                KeyEvent {
                    code: KeyCode::Char('o'),
                    modifiers: KeyModifiers::NONE,
                },
            ),
            Some(KeyEffects {
                open_progress_target: Some(ProgressOpenTarget::OpenUrl(
                    "https://example.com/pull/7".to_string(),
                )),
                ..Default::default()
            }),
        );
    }

    #[rstest]
    fn o_opens_the_selected_checklist_pull_request_url(mut focused_progress_app: App) {
        focused_progress_app.progress_rows = vec![
            work_row(),
            ProgressRow::ChecklistPullRequest {
                batch_id: "batch-1".to_string(),
                row: PrProgressRow {
                    repo: None,
                    session_id: "delegate".to_string(),
                    title: "Example delegated work".to_string(),
                    number: Some(7),
                    url: Some("https://example.com/pull/7".to_string()),
                    pane_id: None,
                    is_ended: true,
                    lookup_unavailable: false,
                    stages: StageMarks {
                        implementation: ProgressMark::Done,
                        diff_review: ProgressMark::User,
                        pr_body: ProgressMark::NotStarted,
                        ci: ProgressMark::NotStarted,
                        merge: ProgressMark::NotStarted,
                    },
                },
            },
        ];
        focused_progress_app.progress_list_state.select(Some(1));

        assert_eq!(
            handle(
                &mut focused_progress_app,
                KeyEvent {
                    code: KeyCode::Char('o'),
                    modifiers: KeyModifiers::NONE,
                },
            ),
            Some(KeyEffects {
                open_progress_target: Some(ProgressOpenTarget::OpenUrl(
                    "https://example.com/pull/7".to_string(),
                )),
                ..Default::default()
            }),
        );
    }

    #[rstest]
    fn o_does_nothing_on_a_work_or_planned_row(mut focused_progress_app: App) {
        focused_progress_app.progress_rows = vec![
            work_row(),
            ProgressRow::PlannedPullRequest {
                batch_id: "batch-1".to_string(),
                checklist_item_id: "item-1".to_string(),
                title: "Future change".to_string(),
            },
        ];

        let actual = [0, 1].map(|index| {
            focused_progress_app.progress_list_state.select(Some(index));
            handle(
                &mut focused_progress_app,
                KeyEvent {
                    code: KeyCode::Char('o'),
                    modifiers: KeyModifiers::NONE,
                },
            )
        });

        assert_eq!(
            actual,
            [Some(KeyEffects::default()), Some(KeyEffects::default())]
        );
    }

    #[test]
    fn stale_rows_cannot_be_navigated_or_opened() {
        let mut app = App::with_sessions(vec![session("previous"), session("selected")]);
        if app.selected_session_id() != Some("selected") {
            app.select_next();
        }
        app.progress_focused = true;
        app.progress_session_id = Some("previous".to_string());
        app.progress_rows = vec![
            work_row(),
            pull_request_row(false, Some("%8"), Some("https://example.com/pull/7")),
        ];
        app.progress_list_state.select(Some(1));

        for key in [KeyCode::Down, KeyCode::Up] {
            handle(
                &mut app,
                KeyEvent {
                    code: key,
                    modifiers: KeyModifiers::NONE,
                },
            );
        }
        let enter_effects = handle(
            &mut app,
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
            },
        );
        let open_effects = handle(
            &mut app,
            KeyEvent {
                code: KeyCode::Char('o'),
                modifiers: KeyModifiers::NONE,
            },
        );

        assert_eq!(
            (
                app.selected_session_id(),
                app.progress_list_state.selected(),
                enter_effects,
                open_effects,
            ),
            (
                Some("selected"),
                Some(1),
                Some(KeyEffects::default()),
                Some(KeyEffects::default()),
            ),
        );
    }

    #[test]
    fn left_returns_focus_to_tasks_without_changing_either_selection() {
        let mut app = App::with_sessions(vec![session("selected")]);
        app.update_row_order(&[None, Some("selected")]);
        app.list_state.select(Some(1));
        app.progress_focused = true;
        app.progress_session_id = Some("selected".to_string());
        app.progress_rows = vec![work_row(), pull_request_row(false, None, None)];
        app.progress_list_state.select(Some(1));
        app.progress_table_state.select(Some(0));

        handle(
            &mut app,
            KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::NONE,
            },
        );

        assert_eq!(
            (
                app.selected_session_id(),
                app.progress_focused,
                app.progress_list_state.selected(),
                app.progress_table_state.selected(),
            ),
            (Some("selected"), false, Some(1), Some(0)),
        );
    }
}
