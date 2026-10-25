use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use crate::commands::agent::tui::app::App;
use crate::commands::agent::tui::progress::{ProgressMark, ProgressRow};
use crate::commands::agent::tui::session_rows::SessionRowKey;

pub(in crate::commands::agent::tui::ui) fn selected_summary_lines(
    app: &App,
    width: usize,
) -> Vec<Line<'static>> {
    let Some(key) = app.selected_row_key() else {
        return Vec::new();
    };
    let mut text = match key {
        SessionRowKey::Progress {
            selection_key,
            session_id,
            ..
        } => app
            .progress_rows
            .iter()
            .find(|row| row.selection_key().as_ref() == Some(selection_key))
            .map(|row| progress_summary(app, row, session_id.as_deref()))
            .unwrap_or_default(),
        SessionRowKey::Task(session_id) => app
            .sessions
            .iter()
            .find(|session| session.session_id == *session_id)
            .map(|session| {
                session_summary(
                    app,
                    session_id,
                    session
                        .agent_status
                        .as_ref()
                        .map(|status| status.note.as_str()),
                )
            })
            .unwrap_or_default(),
        SessionRowKey::UserTurn(session_id) => app
            .sessions
            .iter()
            .find(|session| session.session_id == *session_id)
            .map(|session| {
                let title = app
                    .get_cached_title(session_id)
                    .map(str::to_owned)
                    .unwrap_or_else(|| session.cwd.display().to_string());
                let note = session
                    .agent_status
                    .as_ref()
                    .map(|status| status.note.as_str())
                    .filter(|note| !note.is_empty())
                    .or(session.current_tool.as_deref())
                    .filter(|note| !note.is_empty());
                session_summary_with_title(app, session_id, title, note)
            })
            .unwrap_or_default(),
    };
    if text.is_empty() {
        text = "PR progress".to_string();
    }
    wrap_summary(&text, width.max(1))
        .into_iter()
        .map(|line| Line::from(Span::raw(line)))
        .collect()
}

fn session_summary(app: &App, session_id: &str, note: Option<&str>) -> String {
    session_summary_with_title(app, session_id, session_title(app, session_id), note)
}

fn session_summary_with_title(
    app: &App,
    session_id: &str,
    title: String,
    note: Option<&str>,
) -> String {
    let mut lines = vec![title];
    if let Some(note) = note.filter(|note| !note.is_empty()) {
        lines.push(format!("Waiting: {note}"));
    }
    if app.progress_session_id.as_deref() == Some(session_id)
        && let Some((merged, total)) = app.progress_rows.iter().find_map(|row| match row {
            ProgressRow::ImplementationHeader {
                merged_pr_count,
                total_pr_count,
                ..
            } => Some((merged_pr_count, total_pr_count)),
            _ => None,
        })
    {
        lines.push(format!("PR progress: {merged}/{total} merged"));
    }
    if let Some(row) = app.progress_rows.iter().find_map(|row| match row {
        ProgressRow::PullRequest(row) | ProgressRow::ChecklistPullRequest { row, .. }
            if row.session_id == session_id =>
        {
            Some(row)
        }
        _ => None,
    }) {
        let pr = match row.number {
            Some(number) => format!("PR #{number}: {}", row.title),
            None => format!("PR: {}", row.title),
        };
        lines.push(pr);
        lines.push(format!(
            "PR stages: {}",
            stage_summary(
                row.stages.implementation,
                row.stages.diff_review,
                row.stages.pr_body,
                row.stages.ci,
                row.stages.merge
            )
        ));
    }
    lines.join("\n")
}

fn progress_summary(app: &App, row: &ProgressRow, session_id: Option<&str>) -> String {
    match row {
        ProgressRow::ChecklistPullRequest { row, .. } => {
            let worker = session_id.map(|id| session_title(app, id));
            let pr = match row.number {
                Some(number) => format!("PR #{number}: {}", row.title),
                None => format!("PR: {}", row.title),
            };
            let title = worker.map_or(pr.clone(), |worker| format!("{worker} · {pr}"));
            let note = session_id
                .and_then(|id| app.sessions.iter().find(|session| session.session_id == id))
                .and_then(|session| session.agent_status.as_ref())
                .map(|status| status.note.as_str())
                .filter(|note| !note.is_empty());
            let stages = stage_summary(
                row.stages.implementation,
                row.stages.diff_review,
                row.stages.pr_body,
                row.stages.ci,
                row.stages.merge,
            );
            match note {
                Some(note) => [
                    title,
                    format!("Waiting: {note}"),
                    format!("PR stages: {stages}"),
                ]
                .join("\n"),
                None => format!("{title}\nPR stages: {stages}"),
            }
        }
        ProgressRow::PlannedPullRequest { title, .. } => {
            format!("Planned PR: {title}\nPR stages: not started")
        }
        ProgressRow::ImplementationHeader {
            merged_pr_count,
            total_pr_count,
            ..
        } => format!("Implementation plan · {merged_pr_count}/{total_pr_count} PRs merged"),
        ProgressRow::BatchHeader {
            title,
            merged_pr_count,
            ..
        } => format!("Batch: {title} · {merged_pr_count} PRs merged"),
        ProgressRow::Work(row) => {
            let title = session_title(app, &row.session_id);
            match row.note.as_deref().filter(|note| !note.is_empty()) {
                Some(note) => format!("{title}\nWaiting: {note}"),
                None => title,
            }
        }
        ProgressRow::PullRequest(row) => {
            let title = match row.number {
                Some(number) => format!("PR #{number}: {}", row.title),
                None => format!("PR: {}", row.title),
            };
            format!(
                "{title}\nPR stages: {}",
                stage_summary(
                    row.stages.implementation,
                    row.stages.diff_review,
                    row.stages.pr_body,
                    row.stages.ci,
                    row.stages.merge
                )
            )
        }
        ProgressRow::UnmatchedDelegatesHeader { .. } => "Unmatched delegates".to_string(),
    }
}

fn session_title(app: &App, session_id: &str) -> String {
    let Some(session) = app
        .sessions
        .iter()
        .find(|session| session.session_id == session_id)
    else {
        return session_id.to_string();
    };
    app.task_by_session
        .get(session_id)
        .map(|task| task.task_title.clone())
        .or_else(|| app.get_cached_title(session_id).map(str::to_owned))
        .or_else(|| {
            session
                .cwd
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| session.cwd.display().to_string())
}

fn stage_summary(
    implementation: ProgressMark,
    diff_review: ProgressMark,
    pr_body: ProgressMark,
    ci: ProgressMark,
    merge: ProgressMark,
) -> String {
    format!(
        "implementation {} · review {} · PR body {} · CI {} · merge {}",
        implementation.symbol(),
        diff_review.symbol(),
        pr_body.symbol(),
        ci.symbol(),
        merge.symbol(),
    )
}

fn wrap_summary(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for logical_line in text.split('\n') {
        let mut line = String::new();
        let mut line_width = 0;
        for ch in logical_line.chars() {
            let char_width = ch.width().unwrap_or(0);
            if line_width + char_width > width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line_width = 0;
            }
            line.push(ch);
            line_width += char_width;
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rstest::{fixture, rstest};

    use super::*;
    use crate::commands::agent::tui::app::App;
    use crate::commands::agent::tui::progress::{
        PrProgressRow, ProgressFetchState, ProgressMark, ProgressSelectionKey, StageMarks,
    };
    use crate::commands::agent::tui::session_rows::SessionRowKey;
    use crate::commands::agent::tui::ui::test_support::create_test_session;
    use crate::commands::agent::types::{AgentStatus, SessionStatus};

    #[fixture]
    fn user_turn_worker() -> crate::commands::agent::types::Session {
        let mut worker = create_test_session("worker");
        worker.label = Some("Cached worker title".to_string());
        worker.cwd = PathBuf::from("/home/test/project-fallback");
        worker.status = SessionStatus::WaitingInput;
        worker
            .crit_urls
            .push("https://example.test/review".to_string());
        worker
    }

    fn select_display_row(app: &mut App, key: &SessionRowKey) {
        let rows = app.build_display_rows();
        let row_keys = rows.iter().map(|row| row.row_key()).collect::<Vec<_>>();
        let position = row_keys
            .iter()
            .position(|row_key| row_key.as_ref() == Some(key))
            .expect("requested row should be selectable");
        drop(rows);
        let heights = vec![1; row_keys.len()];
        app.update_rendered_rows(&row_keys, &heights, heights.len());
        app.list_state.select(Some(position));
    }

    #[test]
    fn selected_task_summary_keeps_title_waiting_note_and_plan_progress() {
        let mut design = create_test_session("design");
        design.label = Some("A long design session title".to_string());
        design.agent_status = Some(AgentStatus {
            kind: "waiting".to_string(),
            note: "implementation review".to_string(),
        });
        let mut app = App::with_sessions(vec![design]);
        app.progress_session_id = Some("design".to_string());
        app.progress_rows = vec![ProgressRow::ImplementationHeader {
            merged_pr_count: 2,
            total_pr_count: 5,
            collapsed: false,
        }];
        select_display_row(&mut app, &SessionRowKey::Task("design".to_string()));

        assert_eq!(
            selected_summary_lines(&app, 120),
            vec![
                Line::from("A long design session title"),
                Line::from("Waiting: implementation review"),
                Line::from("PR progress: 2/5 merged"),
            ],
        );
    }

    #[rstest]
    #[case::agent_status_note(
        Some("review the change"),
        Some("Bash cargo test"),
        "review the change"
    )]
    #[case::current_tool_fallback(None, Some("Bash cargo test"), "Bash cargo test")]
    fn selected_user_turn_summary_keeps_title_waiting_note_and_pr_stages(
        mut user_turn_worker: crate::commands::agent::types::Session,
        #[case] status_note: Option<&str>,
        #[case] current_tool: Option<&str>,
        #[case] expected_note: &str,
    ) {
        user_turn_worker.agent_status = status_note.map(|note| AgentStatus {
            kind: "needs_attention".to_string(),
            note: note.to_string(),
        });
        user_turn_worker.current_tool = current_tool.map(str::to_owned);
        let mut app = App::with_sessions(vec![user_turn_worker]);
        app.progress_session_id = Some("worker".to_string());
        app.progress_rows = vec![
            ProgressRow::ImplementationHeader {
                merged_pr_count: 1,
                total_pr_count: 2,
                collapsed: false,
            },
            ProgressRow::PullRequest(PrProgressRow {
                session_id: "worker".to_string(),
                repo: None,
                title: "Implement sample feature".to_string(),
                number: Some(41),
                url: Some("https://example.test/pull/41".to_string()),
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
            }),
        ];
        select_display_row(&mut app, &SessionRowKey::UserTurn("worker".to_string()));

        assert_eq!(
            selected_summary_lines(&app, 120),
            vec![
                Line::from("Cached worker title"),
                Line::from(format!("Waiting: {expected_note}")),
                Line::from("PR progress: 1/2 merged"),
                Line::from("PR #41: Implement sample feature"),
                Line::from("PR stages: implementation ✓ · review ◐ · PR body ● · CI ◷ · merge ○"),
            ],
        );
    }

    #[test]
    fn selected_pr_summary_includes_worker_wait_and_all_stages() {
        let design = create_test_session("design");
        let mut worker = create_test_session("worker");
        worker.label = Some("Implementation session title".to_string());
        worker.agent_status = Some(AgentStatus {
            kind: "waiting".to_string(),
            note: "CI check".to_string(),
        });
        worker.ancestor_session_ids = vec!["design".to_string()];
        let mut app = App::with_sessions(vec![design, worker]);
        app.progress_inline_expanded = true;
        app.progress_session_id = Some("design".to_string());
        app.progress_fetch_states
            .insert("design".to_string(), ProgressFetchState::Ready);
        app.progress_rows = vec![
            ProgressRow::ImplementationHeader {
                merged_pr_count: 0,
                total_pr_count: 1,
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
                row: crate::commands::agent::tui::progress::PrProgressRow {
                    session_id: "worker".to_string(),
                    repo: None,
                    title: "Add a sample feature".to_string(),
                    number: Some(23),
                    url: None,
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
        ];
        let key = SessionRowKey::Progress {
            source_session_id: "design".to_string(),
            selection_key: ProgressSelectionKey::ChecklistItem {
                batch_id: "batch-a".to_string(),
                item_id: "item-a".to_string(),
            },
            session_id: Some("worker".to_string()),
        };
        select_display_row(&mut app, &key);

        assert_eq!(
            selected_summary_lines(&app, 120),
            vec![
                Line::from("Implementation session title · PR #23: Add a sample feature"),
                Line::from("Waiting: CI check"),
                Line::from("PR stages: implementation ✓ · review ◐ · PR body ● · CI ◷ · merge ○"),
            ],
        );
    }

    #[test]
    fn selected_unmatched_pr_session_summary_includes_wait_and_stages() {
        let design = create_test_session("design");
        let mut worker = create_test_session("worker");
        worker.label = Some("Worker title".to_string());
        worker.agent_status = Some(AgentStatus {
            kind: "waiting".to_string(),
            note: "awaiting CI".to_string(),
        });
        worker.ancestor_session_ids = vec!["design".to_string()];
        let mut app = App::with_sessions(vec![design, worker]);
        app.progress_session_id = Some("design".to_string());
        app.progress_rows = vec![ProgressRow::PullRequest(
            crate::commands::agent::tui::progress::PrProgressRow {
                session_id: "worker".to_string(),
                repo: None,
                title: "Unmatched feature".to_string(),
                number: Some(23),
                url: Some("https://example.test/pull/23".to_string()),
                pane_id: None,
                is_ended: false,
                lookup_unavailable: false,
                stages: StageMarks {
                    implementation: ProgressMark::Done,
                    diff_review: ProgressMark::Done,
                    pr_body: ProgressMark::Working,
                    ci: ProgressMark::Waiting,
                    merge: ProgressMark::NotStarted,
                },
            },
        )];
        select_display_row(&mut app, &SessionRowKey::Task("worker".to_string()));

        assert_eq!(
            selected_summary_lines(&app, 120),
            vec![
                Line::from("Worker title"),
                Line::from("Waiting: awaiting CI"),
                Line::from("PR #23: Unmatched feature"),
                Line::from("PR stages: implementation ✓ · review ✓ · PR body ● · CI ◷ · merge ○"),
            ],
        );
    }
}
