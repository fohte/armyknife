use std::collections::HashSet;

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
use crate::commands::agent::user_turn::user_turns;

use super::edit_bar::render_edit_input;
use super::help::{build_help_lines, render_help_lines};
use super::helpers::{DIM_FG, count_statuses, truncate};
use super::session_content::render_main_list;

const HEADER_HEIGHT: u16 = 1;

/// Renders the entire UI.
pub fn render(frame: &mut Frame, app: &mut App) {
    render_with_time(frame, app, Utc::now());
}

pub(super) fn render_with_time(frame: &mut Frame, app: &mut App, now: DateTime<Utc>) {
    let area = frame.area();
    // The top bar (search / rename) is session-view only.
    let has_error = app.error_message.is_some();
    let is_search_mode = app.view == View::Session && app.mode == AppMode::Search;
    let is_edit_mode = app.view == View::Session && matches!(app.mode, AppMode::Edit { .. });
    let has_text_filter = app.view == View::Session && !app.confirmed_query.is_empty();
    let has_drilldown_scope = app.view == View::Session && app.drilldown_scope.is_some();
    let has_input_bar = is_search_mode || has_text_filter || is_edit_mode || has_drilldown_scope;
    let show_top_bar = has_input_bar;

    let help_lines = build_help_lines(app);
    let help_height = help_lines.len() as u16;
    let mut constraints = vec![Constraint::Length(HEADER_HEIGHT)];
    if show_top_bar {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Min(1));
    constraints.push(Constraint::Length(help_height));
    if has_error {
        constraints.push(Constraint::Length(1));
    }
    let areas = Layout::vertical(constraints).split(area);

    render_header(frame, areas[0], app);
    let mut area_index = 1;
    if show_top_bar {
        render_top_bar(frame, areas[area_index], app);
        area_index += 1;
    }
    render_main_list(frame, areas[area_index], app, now);
    area_index += 1;
    render_help_lines(frame, areas[area_index], help_lines);
    area_index += 1;
    if has_error {
        render_error(
            frame,
            areas[area_index],
            app.error_message.as_deref().unwrap_or(""),
        );
    }
}

/// Dispatches the bar rendered above the session list: the rename bar
/// while `AppMode::Edit` is active, the search bar otherwise (live query
/// while searching, or the confirmed filter query while browsing a
/// filtered list).
fn render_top_bar(frame: &mut Frame, area: Rect, app: &App) {
    render_input_bar(frame, area, app);
}

fn render_input_bar(frame: &mut Frame, area: Rect, app: &App) {
    if matches!(app.mode, AppMode::Edit { .. }) {
        render_edit_input(frame, area, app);
    } else {
        render_search_input(frame, area, app);
    }
}

/// Renders the single-line header: title on the left, a compact status
/// summary right-aligned. `idle` combines stopped, ended, and paused sessions;
/// user turns are excluded from this count.
fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let counts = header_counts(app);

    let title = " agent watch";
    let needs_you = format!("{} needs you", counts.needs_you);
    let running_text = format!("{} running", counts.running);
    let idle_text = format!("{} idle", counts.idle);
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
        Span::styled(idle_text, Style::default().fg(DIM_FG)),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}

#[derive(Debug, PartialEq, Eq)]
struct HeaderCounts {
    needs_you: usize,
    running: usize,
    idle: usize,
}

fn header_counts(app: &App) -> HeaderCounts {
    let sessions = app.filtered_sessions();
    let turns = user_turns(sessions.iter().copied(), &app.agent_config);
    let turn_session_ids: HashSet<&str> = turns
        .iter()
        .map(|turn| turn.session.session_id.as_str())
        .collect();
    let (running, stopped, paused) = count_statuses(
        sessions
            .iter()
            .copied()
            .filter(|session| !turn_session_ids.contains(session.session_id.as_str())),
    );

    HeaderCounts {
        needs_you: turns.len(),
        running,
        idle: stopped + paused,
    }
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
    spans.push(Span::styled(count_str, Style::default().fg(DIM_FG)));

    let search_text = Line::from(spans);
    let search = Paragraph::new(search_text);
    frame.render_widget(search, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::tui::progress::ProgressRow;
    use crate::commands::agent::tui::ui::test_support::{
        create_test_session, render_buffer, render_to_string, render_to_string_with, wt_row,
    };
    use crate::commands::agent::types::{AgentStatus, Session, SessionStatus};
    use indoc::indoc;
    use rstest::rstest;

    #[test]
    fn test_render_header_status_summary() {
        let now = Utc::now();

        let mut running = create_test_session("s1");
        running.status = SessionStatus::Running;
        let mut waiting = create_test_session("s2");
        waiting.status = SessionStatus::WaitingInput;
        waiting
            .crit_urls
            .push("https://example.com/review".to_string());
        let mut paused = create_test_session("s3");
        paused.status = SessionStatus::Paused;
        let mut stopped = create_test_session("s4");
        stopped.status = SessionStatus::Stopped;

        let sessions = vec![running, waiting, paused, stopped];
        let buffer = render_buffer(&sessions, Some(0), now, 80, 20);
        let output = render_to_string(&sessions, Some(0), now, 80, 20);
        let header_line = output.lines().next().unwrap();
        let idle_start = header_line
            .find("2 idle")
            .map(|byte_index| header_line[..byte_index].chars().count() as u16)
            .unwrap();

        assert_eq!(
            header_line,
            " agent watch                                    1 needs you · 1 running · 2 idle"
        );
        assert_eq!(buffer[(idle_start, 0)].fg, DIM_FG);
    }

    #[test]
    fn test_session_view_omits_selected_row_summary_and_keeps_help_bar() {
        let now = Utc::now();
        let mut worker = create_test_session("worker");
        worker.label = Some("Selected worker".to_string());
        worker.agent_status = Some(AgentStatus {
            kind: "waiting".to_string(),
            note: "CI check".to_string(),
        });
        let sessions = vec![worker];
        let output = render_to_string_with(&sessions, Some(1), now, 80, 8, |app| {
            app.progress_session_id = Some("worker".to_string());
            app.progress_rows = vec![ProgressRow::ImplementationHeader {
                merged_pr_count: 2,
                total_pr_count: 5,
                collapsed: false,
            }];
        });
        let expected = indoc! {"
             agent watch                                    0 needs you · 1 running · 0 idle
             ── TASKS ──────────────────────────────────────────────────────────────────────
            >● project                 Selected worker                              just now




             ?: keys   /: search   Tab: toggle progress   q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_header_counts_use_filtered_sessions_and_exclude_user_turns_from_idle() {
        let mut decide = create_test_session("decide");
        decide.status = SessionStatus::Stopped;
        decide.agent_status = Some(AgentStatus {
            kind: "needs_attention".to_string(),
            note: "choose an option".to_string(),
        });
        let mut running = create_test_session("running");
        running.status = SessionStatus::Running;
        let mut paused = create_test_session("paused");
        paused.status = SessionStatus::Paused;
        let mut app = App::with_sessions(vec![decide, running, paused]);

        assert_eq!(
            header_counts(&app),
            HeaderCounts {
                needs_you: 1,
                running: 1,
                idle: 1,
            }
        );

        app.filtered_indices = vec![0, 2];
        assert_eq!(
            header_counts(&app),
            HeaderCounts {
                needs_you: 1,
                running: 0,
                idle: 1,
            }
        );
    }

    #[rstest]
    #[case::session_view_default(View::Session, false, false, vec![
        " ?: keys   /: search   Tab: toggle progress   q: quit".to_string(),
    ])]
    #[case::session_view_expanded(View::Session, true, false, vec![
        "  j/k: move  f: focus  r: resume  p: preview  t: open task  d: delete".to_string(),
        "  1-9: quick  /: search  h/←: parent / fold plan  →/l: drill down / expand plan".to_string(),
        "  C-r/w/s/p: filter  o: open PR/crit  q: quit".to_string(),
        "  Tab: toggle progress".to_string(),
    ])]
    #[case::progress_view(View::Session, false, true, vec![
        " ?: keys   /: search   Tab: toggle progress   q: quit".to_string(),
    ])]
    fn test_help_bar_default_vs_expanded(
        #[case] view: View,
        #[case] show_help: bool,
        #[case] progress_expanded: bool,
        #[case] expected_lines: Vec<String>,
    ) {
        let now = Utc::now();
        let sessions: Vec<Session> = vec![];
        let height = 8 + expected_lines.len() as u16;
        let output = render_to_string_with(&sessions, None, now, 120, height, |app| {
            app.view = view;
            app.show_help = show_help;
            app.progress_inline_source_id = progress_expanded.then(|| "source-session".to_string());
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
