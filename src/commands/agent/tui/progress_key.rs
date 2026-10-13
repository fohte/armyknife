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
        PrProgressRow, ProgressMark, ProgressOpenTarget, ProgressRow, StageMarks, WorkProgressRow,
    };
    use crate::commands::agent::types::{Engine, Session, SessionStatus};

    fn session(session_id: &str) -> Session {
        let now = Utc::now();
        Session {
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
