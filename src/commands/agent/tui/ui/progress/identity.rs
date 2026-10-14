use ratatui::{style::Style, text::Span};
use unicode_width::UnicodeWidthStr;

use crate::commands::agent::tui::{
    app::App,
    progress::ProgressMark,
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
pub(super) enum IdentityStatus<'a> {
    MissingSession,
    Planned,
    Session(&'a Session),
}

pub(super) fn identity_status<'a>(app: &'a App, session_id: &str) -> IdentityStatus<'a> {
    app.sessions
        .iter()
        .find(|session| session.session_id == session_id)
        .map_or(IdentityStatus::MissingSession, IdentityStatus::Session)
}

pub(super) fn progress_row_identity(
    app: &App,
    status: IdentityStatus<'_>,
    number: String,
    indent: &str,
) -> Vec<Span<'static>> {
    let session = match status {
        IdentityStatus::Session(session) => Some(session),
        IdentityStatus::MissingSession | IdentityStatus::Planned => None,
    };
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
        status_span(app, status),
        Span::raw(" "),
        Span::styled(repo, Style::default().fg(DIM_FG)),
        Span::styled(pad_number(&number), number_style),
    ]
}

fn status_span(app: &App, status: IdentityStatus<'_>) -> Span<'static> {
    match status {
        IdentityStatus::MissingSession => Span::styled("-", Style::default().fg(DIM_FG)),
        IdentityStatus::Planned => Span::styled("○", mark_style(ProgressMark::NotStarted)),
        IdentityStatus::Session(session) => {
            let (glyph, color) = mark_glyph(session_task_mark(app, session), session);
            Span::styled(glyph, Style::default().fg(color))
        }
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
        let span = status_span(&app, IdentityStatus::Session(&session));

        assert_eq!(
            (span.content.to_string(), span.style),
            ("⏸".to_string(), Style::default().fg(DIM_FG)),
        );
    }
}
