use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

use crate::commands::agent::tui::app::{App, AppMode, View};
use crate::commands::agent::tui::tq_sidebar::SidebarSelection;

use super::edit_bar::render_edit_input;
use super::help::{build_help_lines, render_help_lines};
use super::helpers::{count_statuses, truncate};
use super::session_content::{render_main_list, render_sidebar_scope};
use super::tq_sidebar::minimum_total_width;

const HEADER_HEIGHT: u16 = 1;

/// Renders the entire UI.
pub fn render(frame: &mut Frame, app: &mut App) {
    render_with_time(frame, app, Utc::now());
}

pub(super) fn render_with_time(frame: &mut Frame, app: &mut App, now: DateTime<Utc>) {
    let area = frame.area();
    app.set_sidebar_available(app.view == View::Session && area.width >= minimum_total_width());

    // The top bar (search / rename) is session-view only.
    let has_error = app.error_message.is_some();
    let is_search_mode = app.view == View::Session && app.mode == AppMode::Search;
    let is_edit_mode = app.view == View::Session && matches!(app.mode, AppMode::Edit { .. });
    let has_text_filter = app.view == View::Session && !app.confirmed_query.is_empty();
    let has_drilldown_scope = app.view == View::Session && app.drilldown_scope.is_some();
    let has_sidebar_scope =
        app.view == View::Session && app.sidebar_selection != SidebarSelection::All;
    let has_input_bar = is_search_mode || has_text_filter || is_edit_mode || has_drilldown_scope;
    let show_top_bar = has_input_bar || has_sidebar_scope;
    let top_bar_height = if has_input_bar && has_sidebar_scope {
        2
    } else {
        1
    };

    let help_lines = build_help_lines(app);
    let help_height = help_lines.len() as u16;

    let layouts: Vec<Constraint> = match (show_top_bar, has_error) {
        (true, true) => vec![
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Length(top_bar_height), // Search / rename / tq scope
            Constraint::Min(1),                 // Session list
            Constraint::Length(help_height),
            Constraint::Length(1), // Error
        ],
        (true, false) => vec![
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Length(top_bar_height), // Search / rename / tq scope
            Constraint::Min(1),                 // Session list
            Constraint::Length(help_height),
        ],
        (false, true) => vec![
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Min(1), // Session list
            Constraint::Length(help_height),
            Constraint::Length(1), // Error
        ],
        (false, false) => vec![
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Min(1), // Session list
            Constraint::Length(help_height),
        ],
    };

    let areas = Layout::vertical(layouts).split(area);

    render_header(frame, areas[0], app);

    match (show_top_bar, has_error) {
        (true, true) => {
            render_top_bar(frame, areas[1], app);
            render_main_list(frame, areas[2], app, now);
            render_help_lines(frame, areas[3], help_lines);
            render_error(frame, areas[4], app.error_message.as_deref().unwrap_or(""));
        }
        (true, false) => {
            render_top_bar(frame, areas[1], app);
            render_main_list(frame, areas[2], app, now);
            render_help_lines(frame, areas[3], help_lines);
        }
        (false, true) => {
            render_main_list(frame, areas[1], app, now);
            render_help_lines(frame, areas[2], help_lines);
            render_error(frame, areas[3], app.error_message.as_deref().unwrap_or(""));
        }
        (false, false) => {
            render_main_list(frame, areas[1], app, now);
            render_help_lines(frame, areas[2], help_lines);
        }
    }
}

/// Dispatches the bar rendered above the session list: the rename bar
/// while `AppMode::Edit` is active, the search bar otherwise (live query
/// while searching, or the confirmed filter query while browsing a
/// filtered list).
fn render_top_bar(frame: &mut Frame, area: Rect, app: &App) {
    let has_input_bar = app.mode == AppMode::Search
        || !app.confirmed_query.is_empty()
        || matches!(app.mode, AppMode::Edit { .. })
        || app.drilldown_scope.is_some();
    let has_sidebar_scope = app.sidebar_selection != SidebarSelection::All;
    if has_input_bar && has_sidebar_scope && area.height > 1 {
        let [input_area, scope_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
        render_input_bar(frame, input_area, app);
        render_sidebar_scope(frame, scope_area, app);
    } else if has_input_bar {
        render_input_bar(frame, area, app);
    } else {
        render_sidebar_scope(frame, area, app);
    }
}

fn render_input_bar(frame: &mut Frame, area: Rect, app: &App) {
    if matches!(app.mode, AppMode::Edit { .. }) {
        render_edit_input(frame, area, app);
    } else {
        render_search_input(frame, area, app);
    }
}

/// Renders the single-line header: title on the left, a compact status
/// summary right-aligned. `idle` folds together Stopped and Paused since
/// neither needs the user's attention.
fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let (running, waiting, stopped, paused) = count_statuses(&app.sessions);
    let idle = stopped + paused;

    let title = " agent watch";
    let needs_you = format!("{waiting} needs you");
    let running_text = format!("{running} running");
    let idle_text = format!("{idle} idle");
    let summary = format!("{needs_you} · {running_text} · {idle_text}");

    let term_width = area.width as usize;
    let gap = term_width
        .saturating_sub(title.width())
        .saturating_sub(summary.width());

    let line = Line::from(vec![
        Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(gap)),
        Span::styled(
            needs_you,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" · "),
        Span::styled(
            running_text,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" · "),
        Span::styled(idle_text, Style::default().fg(Color::DarkGray)),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}

fn render_error(frame: &mut Frame, area: Rect, message: &str) {
    let error_text = Line::from(vec![
        Span::styled(
            "  Error: ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(message, Style::default().fg(Color::Red)),
    ]);

    let error = Paragraph::new(error_text);
    frame.render_widget(error, area);
}

/// Renders the search input bar.
fn render_search_input(frame: &mut Frame, area: Rect, app: &App) {
    let filtered_count = app.filtered_indices.len();
    let total_count = app.sessions.len();
    let count_str = format!("({}/{})", filtered_count, total_count);
    let term_width = area.width as usize;

    let is_search_mode = app.mode == AppMode::Search;

    // Use different query based on mode
    let query = if is_search_mode {
        &app.search_query
    } else {
        &app.confirmed_query
    };

    // Calculate available width for the search query
    let prefix = match &app.drilldown_scope {
        Some(root_id) => {
            let title = app.get_cached_title(root_id).unwrap_or(root_id.as_str());
            format!("  \u{25b8} {title} \u{203a} /")
        }
        None => "  /".to_string(),
    };
    let cursor_str = if is_search_mode { "_" } else { "" };
    let count_width = count_str.len();
    // Terminal-cell width, not byte length -- a drill-down scope's prefix
    // can embed a session title with wide/multi-byte characters.
    let prefix_width = prefix.width();
    let fixed_width = prefix_width + cursor_str.len() + count_width + 2; // +2 for spacing
    let query_max_width = term_width.saturating_sub(fixed_width);

    // Truncate query if needed
    let display_query = truncate(query, query_max_width);

    // Calculate padding to right-align the count
    let content_width = prefix_width + display_query.width() + cursor_str.len();
    let padding_width = term_width.saturating_sub(content_width + count_width + 2);
    let padding = " ".repeat(padding_width);

    let mut spans = vec![Span::styled(prefix, Style::default().fg(Color::Yellow))];

    spans.push(Span::styled(display_query, Style::default()));

    // Only show cursor in search mode
    if is_search_mode {
        spans.push(Span::styled(
            cursor_str,
            Style::default().add_modifier(Modifier::SLOW_BLINK),
        ));
    }

    spans.push(Span::raw(padding));
    spans.push(Span::styled(
        count_str,
        Style::default().fg(Color::DarkGray),
    ));

    let search_text = Line::from(spans);
    let search = Paragraph::new(search_text);
    frame.render_widget(search, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::tui::ui::test_support::{
        create_test_session, render_to_string, render_to_string_with, wt_row,
    };
    use crate::commands::agent::types::{Session, SessionStatus};
    use rstest::rstest;

    #[test]
    fn test_render_header_status_summary() {
        let now = Utc::now();

        let mut running = create_test_session("s1");
        running.status = SessionStatus::Running;
        let mut waiting = create_test_session("s2");
        waiting.status = SessionStatus::WaitingInput;
        let mut paused = create_test_session("s3");
        paused.status = SessionStatus::Paused;
        let mut stopped = create_test_session("s4");
        stopped.status = SessionStatus::Stopped;

        let sessions = vec![running, waiting, paused, stopped];
        let output = render_to_string(&sessions, Some(0), now, 80, 20);
        let header_line = output.lines().next().unwrap();

        assert_eq!(
            header_line,
            " agent watch                                    1 needs you · 1 running · 2 idle"
        );
    }

    #[rstest]
    #[case::session_view_default(View::Session, false, vec![
        " ?: keys   /: search   q: quit".to_string(),
    ])]
    #[case::session_view_expanded(View::Session, true, vec![
        "  j/k: move  f: focus  r: resume  p: preview  t: open task  d: delete".to_string(),
        "  1-9: quick  /: search  h/←: parent  →/l: drill down".to_string(),
        "  C-r/w/s/p: filter  o: open crit  q: quit".to_string(),
    ])]
    fn test_help_bar_default_vs_expanded(
        #[case] view: View,
        #[case] show_help: bool,
        #[case] expected_lines: Vec<String>,
    ) {
        let now = Utc::now();
        let sessions: Vec<Session> = vec![];
        let height = 8 + expected_lines.len() as u16;
        let output = render_to_string_with(&sessions, None, now, 80, height, |app| {
            app.view = view;
            app.show_help = show_help;
        });

        let actual_lines: Vec<&str> = output
            .lines()
            .skip(output.lines().count() - expected_lines.len())
            .collect();
        assert_eq!(actual_lines, expected_lines);
    }

    #[test]
    fn test_confirm_mode_help_bar_is_single_line() {
        let now = Utc::now();
        let sessions = vec![create_test_session("s1")];
        let output = render_to_string_with(&sessions, Some(0), now, 80, 9, |app| {
            app.mode = AppMode::Confirm {
                session_id: "s1".to_string(),
                is_alive: false,
                worktree_cleanup: None,
            };
        });

        let help_line = output.lines().last().unwrap();
        assert_eq!(help_line, "  Delete session? y: yes  n/Esc: cancel");
    }

    #[test]
    fn test_search_bar_shows_drilldown_scope_title_prefix() {
        let now = Utc::now();
        let sessions = vec![create_test_session("root")];
        let output = render_to_string_with(&sessions, Some(1), now, 80, 9, |app| {
            app.drilldown_scope = Some("root".to_string());
        });

        let top_bar_line = output.lines().nth(1).unwrap();
        assert_eq!(
            top_bar_line,
            "  \u{25b8} project \u{203a} /                                                          (1/1)"
        );
    }

    #[test]
    fn test_search_bar_right_aligns_count_with_multibyte_scope_title() {
        // Regression guard: the padding math must key off the prefix's
        // terminal-cell width, not its UTF-8 byte length, or a scope title
        // with multi-byte characters pushes the right-aligned `(n/n)` count
        // short of its correct column. Cyrillic is single-column-wide per
        // character but 2 bytes each in UTF-8, so a byte-length-based
        // computation would overcount width without needing any
        // double-width (CJK) glyph, which the test backend renders with an
        // extra filler cell that would otherwise confound the width math
        // this test is checking. With an empty query and no truncation, the
        // bar's trimmed rendered width is always `term_width - 2` (the
        // 2-column spacing built into the padding math) when the width
        // accounting is correct, regardless of what characters make up the
        // prefix.
        let now = Utc::now();
        let mut root = create_test_session("root");
        root.label = Some("привет".to_string());
        let sessions = vec![root];
        let output = render_to_string_with(&sessions, Some(1), now, 80, 9, |app| {
            app.drilldown_scope = Some("root".to_string());
        });

        let top_bar_line = output.lines().nth(1).unwrap();
        assert!(
            top_bar_line.ends_with("(1/1)"),
            "count must stay at the end of the bar, got: {top_bar_line:?}"
        );
        assert_eq!(top_bar_line.width(), 78);
    }

    #[test]
    fn test_clean_view_help_bar_unchanged() {
        let now = Utc::now();
        let output = render_to_string_with(&[], None, now, 80, 16, |app| {
            app.set_worktrees(vec![wt_row(
                "armyknife",
                "feat/a",
                "feat-a",
                "/tmp/armyknife/.worktrees/feat-a",
            )]);
            app.enter_clean_view();
        });

        let lines: Vec<&str> = output.lines().collect();
        let help_lines = &lines[lines.len() - 2..];
        assert_eq!(
            help_lines,
            &[
                "  j/k: move  Enter: toggle / focus session  y: run  n/Esc/q: cancel",
                "  Nothing to clean. n/Esc/q: back",
            ]
        );
    }
}
