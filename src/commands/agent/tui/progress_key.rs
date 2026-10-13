use crossterm::event::{KeyCode, KeyModifiers};

use super::KeyEffects;
use super::app::App;
use super::event::KeyEvent;
use super::progress::{ProgressRow, open_target};

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
            let selected_row = app
                .progress_list_state
                .selected()
                .and_then(|index| app.progress_rows.get(index));
            let collapsed = match selected_row {
                Some(ProgressRow::ImplementationHeader { collapsed, .. })
                | Some(ProgressRow::BatchHeader { collapsed, .. }) => Some(*collapsed),
                _ => None,
            };
            if let Some(collapsed) = collapsed {
                app.set_progress_batch_collapsed(!collapsed);
            } else {
                return Some(KeyEffects {
                    open_progress_target: app
                        .progress_list_state
                        .selected()
                        .and_then(|index| app.progress_rows.get(index))
                        .and_then(open_target),
                    ..Default::default()
                });
            }
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
    use rstest::rstest;

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

    #[test]
    fn batch_keys_select_headings_and_skip_planned_rows() {
        let selected = session("selected");
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
                delegated: fetched_delegates,
                implementation: Some(plan),
                github_error: None,
            }),
        );
        app.progress_list_state.select(Some(2));
        app.progress_table_state.select(Some(1));

        let send_key = |app: &mut App, code| {
            handle(
                app,
                KeyEvent {
                    code,
                    modifiers: KeyModifiers::NONE,
                },
            );
        };
        send_key(&mut app, KeyCode::Char('h'));
        let collapsed = (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        );
        send_key(&mut app, KeyCode::Char('l'));
        let expanded = (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        );
        send_key(&mut app, KeyCode::Enter);
        let collapsed_by_enter = (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        );
        send_key(&mut app, KeyCode::Enter);
        let expanded_by_enter = (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        );
        send_key(&mut app, KeyCode::Char('j'));
        send_key(&mut app, KeyCode::Char('j'));
        let navigated = (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        );
        send_key(&mut app, KeyCode::Char('h'));
        let collapsed_from_pr = (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        );
        send_key(&mut app, KeyCode::Char('k'));
        send_key(&mut app, KeyCode::Char('h'));
        let implementation_collapsed = (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        );
        send_key(&mut app, KeyCode::Char('l'));
        let implementation_expanded = (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        );
        send_key(&mut app, KeyCode::Enter);
        let implementation_collapsed_by_enter = (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        );
        send_key(&mut app, KeyCode::Enter);
        let implementation_expanded_by_enter = (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        );

        let work = ProgressRow::Work(WorkProgressRow {
            session_id: "selected".to_string(),
            role: "session".to_string(),
            mark: ProgressMark::Working,
            status_label: None,
            note: None,
            pane_id: None,
        });
        let active_pr = ProgressRow::ChecklistPullRequest {
            batch_id: "batch-1".to_string(),
            row: PrProgressRow {
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
        };
        let unstarted_pr = ProgressRow::PlannedPullRequest {
            batch_id: "batch-1".to_string(),
            checklist_item_id: "item-2".to_string(),
            title: "Future change".to_string(),
        };
        let implementation_header = ProgressRow::ImplementationHeader {
            merged_pr_count: 0,
            total_pr_count: 2,
            collapsed: false,
        };
        let batch_header = ProgressRow::BatchHeader {
            batch_id: "batch-1".to_string(),
            title: "Batch 1".to_string(),
            merged_pr_count: 0,
            collapsed: false,
        };
        let collapsed_batch_rows = vec![
            work.clone(),
            implementation_header.clone(),
            ProgressRow::BatchHeader {
                batch_id: "batch-1".to_string(),
                title: "Batch 1".to_string(),
                merged_pr_count: 0,
                collapsed: true,
            },
        ];
        let expanded_batch_rows = vec![
            work.clone(),
            implementation_header.clone(),
            batch_header.clone(),
            active_pr.clone(),
            unstarted_pr.clone(),
        ];
        let collapsed_implementation_rows = vec![
            work.clone(),
            ProgressRow::ImplementationHeader {
                merged_pr_count: 0,
                total_pr_count: 2,
                collapsed: true,
            },
        ];
        let expanded_implementation_rows = vec![
            work.clone(),
            implementation_header.clone(),
            ProgressRow::BatchHeader {
                batch_id: "batch-1".to_string(),
                title: "Batch 1".to_string(),
                merged_pr_count: 0,
                collapsed: true,
            },
        ];

        assert_eq!(
            [
                collapsed,
                expanded,
                collapsed_by_enter,
                expanded_by_enter,
                navigated,
                collapsed_from_pr,
                implementation_collapsed,
                implementation_expanded,
                implementation_collapsed_by_enter,
                implementation_expanded_by_enter,
            ],
            [
                (collapsed_batch_rows.clone(), Some(2), Some(1)),
                (expanded_batch_rows.clone(), Some(2), Some(1)),
                (collapsed_batch_rows.clone(), Some(2), Some(1)),
                (expanded_batch_rows.clone(), Some(2), Some(1)),
                (expanded_batch_rows.clone(), Some(3), Some(2)),
                (collapsed_batch_rows, Some(2), Some(1)),
                (collapsed_implementation_rows.clone(), Some(1), Some(0)),
                (expanded_implementation_rows.clone(), Some(1), Some(0)),
                (collapsed_implementation_rows, Some(1), Some(0)),
                (expanded_implementation_rows, Some(1), Some(0)),
            ],
        );
    }

    #[rstest]
    #[case::focus_running_delegate(false, Some("%8"), Some("https://example.com/pull/7"), Some(ProgressOpenTarget::FocusPane("%8".to_string())))]
    #[case::open_pr_after_delegate_ends(true, Some("%8"), Some("https://example.com/pull/7"), Some(ProgressOpenTarget::OpenUrl("https://example.com/pull/7".to_string())))]
    #[case::ignore_row_without_target(false, None, None, None)]
    fn enter_opens_the_progress_row_target(
        #[case] is_ended: bool,
        #[case] pane_id: Option<&str>,
        #[case] url: Option<&str>,
        #[case] expected_target: Option<ProgressOpenTarget>,
    ) {
        let mut app = App::with_sessions(vec![session("selected")]);
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
        let effects = handle(
            &mut app,
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
            },
        );

        assert_eq!(
            (
                app.selected_session_id(),
                app.progress_list_state.selected(),
                effects
            ),
            (Some("selected"), Some(1), Some(KeyEffects::default())),
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
