use crossterm::event::{KeyCode, KeyModifiers};

use super::KeyEffects;
use super::app::App;
use super::event::KeyEvent;
use super::progress::open_target;

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
        (KeyCode::Char('h'), KeyModifiers::NONE) if current_rows => {
            app.set_progress_batch_collapsed(true);
        }
        (KeyCode::Char('l'), KeyModifiers::NONE) if current_rows => {
            app.set_progress_batch_collapsed(false);
        }
        (KeyCode::Enter, _) if current_rows => {
            return Some(KeyEffects {
                open_progress_target: app
                    .progress_list_state
                    .selected()
                    .and_then(|index| app.progress_rows.get(index))
                    .and_then(open_target),
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
    use rstest::rstest;

    use super::*;
    use crate::commands::agent::tui::progress::{
        FetchedDelegate, FetchedProgress, PrProgressRow, ProgressMark, ProgressOpenTarget,
        ProgressRow, StageMarks, WorkProgressRow,
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
    fn batch_keys_collapse_and_expand_without_selecting_headings_or_planned_rows() {
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
        send_key(&mut app, KeyCode::Char('j'));
        send_key(&mut app, KeyCode::Char('j'));
        let navigated = (
            app.progress_rows.clone(),
            app.progress_list_state.selected(),
            app.progress_table_state.selected(),
        );
        send_key(&mut app, KeyCode::Char('h'));
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

        assert_eq!(
            [
                collapsed,
                expanded,
                navigated,
                implementation_collapsed,
                implementation_expanded,
            ],
            [
                (
                    vec![
                        work.clone(),
                        implementation_header.clone(),
                        ProgressRow::BatchHeader {
                            batch_id: "batch-1".to_string(),
                            title: "Batch 1".to_string(),
                            merged_pr_count: 0,
                            collapsed: true,
                        },
                    ],
                    Some(0),
                    None,
                ),
                (
                    vec![
                        work.clone(),
                        implementation_header.clone(),
                        active_pr.clone(),
                        unstarted_pr.clone(),
                    ],
                    Some(0),
                    None,
                ),
                (
                    vec![work, implementation_header, active_pr, unstarted_pr],
                    Some(2),
                    Some(1),
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
                        ProgressRow::ImplementationHeader {
                            merged_pr_count: 0,
                            total_pr_count: 2,
                            collapsed: true,
                        },
                    ],
                    Some(0),
                    None,
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
                        ProgressRow::ImplementationHeader {
                            merged_pr_count: 0,
                            total_pr_count: 2,
                            collapsed: false,
                        },
                        ProgressRow::BatchHeader {
                            batch_id: "batch-1".to_string(),
                            title: "Batch 1".to_string(),
                            merged_pr_count: 0,
                            collapsed: true,
                        },
                    ],
                    Some(0),
                    None,
                ),
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
}
