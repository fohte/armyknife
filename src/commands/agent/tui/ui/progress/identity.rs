use ratatui::{style::Style, text::Span};
use unicode_width::UnicodeWidthStr;

use crate::commands::agent::tui::{
    app::App,
    progress::{PrProgressRow, ProgressMark},
    session_rows::{TaskMark, is_related_task, task_mark},
};
use crate::commands::agent::types::Session;
use crate::commands::agent::user_turn::user_turns;

use super::super::{
    helpers::{DIM_FG, REPO_COLUMN_WIDTH, get_session_info, truncate},
    session_list::{mark_glyph, task_number::task_number_style},
};
use super::columns::mark_style;

const PROGRESS_NUMBER_COLUMN_WIDTH: usize = 7;

#[derive(Clone, Copy)]
pub(super) enum IdentityStatus {
    MissingSession,
    Planned,
    Session(TaskMark),
}

pub(super) fn progress_row_identity(
    app: &App,
    session: Option<&Session>,
    status: IdentityStatus,
    number: String,
    indent: &str,
) -> Vec<Span<'static>> {
    let repo = session.map_or_else(
        || " ".repeat(REPO_COLUMN_WIDTH),
        |session| {
            let repo = app
                .get_cached_worktree_labels(&session.cwd)
                .map_or("", |(repo, _)| repo);
            let repo = truncate(&get_session_info(session, repo), REPO_COLUMN_WIDTH);
            format!(
                "{repo}{}",
                " ".repeat(REPO_COLUMN_WIDTH.saturating_sub(repo.width()))
            )
        },
    );
    let number_style = match app
        .task_by_session
        .get(session.map_or("", |session| session.session_id.as_str()))
    {
        Some(task) => {
            let cursor_task = app
                .selected_session_id()
                .and_then(|id| app.task_by_session.get(id));
            task_number_style(is_related_task(cursor_task, Some(task)), task.is_closed)
        }
        None => Style::default().fg(DIM_FG),
    };
    vec![
        Span::raw(indent.to_string()),
        status_span(status, session),
        Span::raw(" "),
        Span::styled(repo, Style::default().fg(DIM_FG)),
        Span::styled(pad_number(&number), number_style),
    ]
}

fn status_span(status: IdentityStatus, session: Option<&Session>) -> Span<'static> {
    match status {
        IdentityStatus::MissingSession => Span::styled("-", Style::default().fg(DIM_FG)),
        IdentityStatus::Planned => Span::styled("○", mark_style(ProgressMark::NotStarted)),
        IdentityStatus::Session(mark) => match session {
            Some(session) => {
                let (glyph, color) = mark_glyph(mark, session);
                Span::styled(glyph, Style::default().fg(color))
            }
            None => Span::styled("-", Style::default().fg(DIM_FG)),
        },
    }
}

pub(super) fn session_task_mark(app: &App, session: &Session) -> TaskMark {
    let has_user_turn = !user_turns(std::iter::once(session), &app.agent_config).is_empty();
    task_mark(session, has_user_turn, &app.agent_config)
}

pub(super) fn task_number(app: &App, session_id: &str) -> String {
    app.task_by_session
        .get(session_id)
        .map_or_else(String::new, |task| format!("#{}", task.task_number))
}

fn pad_number(number: &str) -> String {
    let padding = PROGRESS_NUMBER_COLUMN_WIDTH
        .saturating_sub(number.width())
        .max(usize::from(!number.is_empty()));
    format!("{number}{}", " ".repeat(padding))
}

pub(super) fn pr_session<'a>(app: &'a App, row: &PrProgressRow) -> Option<&'a Session> {
    app.sessions
        .iter()
        .find(|session| session.session_id == row.session_id)
}

#[cfg(test)]
mod tests {
    use ratatui::style::Style;

    use super::*;
    use crate::commands::agent::tui::{app::App, ui::test_support::create_test_session};

    #[test]
    fn progress_identity_uses_the_tasks_glyph_for_unclassified_sessions() {
        let mut session = create_test_session("paused");
        session.status = crate::commands::agent::types::SessionStatus::Paused;
        let app = App::with_sessions(vec![session.clone()]);
        let status = IdentityStatus::Session(session_task_mark(&app, &session));
        let span = status_span(status, Some(&session));

        assert_eq!(
            (span.content.to_string(), span.style),
            ("⏸".to_string(), Style::default().fg(DIM_FG)),
        );
    }
}
