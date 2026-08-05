use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

use crate::commands::cc::tui::app::{App, AppMode, View};
use crate::commands::cc::tui::worktree_view::WorktreeMode;

use super::clean_list::render_clean_list;
use super::edit_bar::render_edit_input;
use super::helpers::{count_statuses, truncate};
use super::session_list::render_session_list;
use super::worktree_list::render_worktree_list;

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
    let show_top_bar = is_search_mode || has_text_filter || is_edit_mode || has_drilldown_scope;

    let help_lines = build_help_lines(app);
    let help_height = help_lines.len() as u16;

    let layouts: Vec<Constraint> = match (show_top_bar, has_error) {
        (true, true) => vec![
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Length(1), // Top bar (search / rename)
            Constraint::Min(1),    // Session list
            Constraint::Length(help_height),
            Constraint::Length(1), // Error
        ],
        (true, false) => vec![
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Length(1), // Top bar (search / rename)
            Constraint::Min(1),    // Session list
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

    let title = " cc watch";
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

/// Dispatch list rendering on the active view.
fn render_main_list(frame: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
    match app.view {
        View::Session => render_session_list(frame, area, app, now),
        View::Worktree => render_worktree_list(frame, area, app, now),
        View::Clean => render_clean_list(frame, area, app, now),
    }
}

/// Renders help-bar content that was already built by `build_help_lines`.
fn render_help_lines(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    let help = Paragraph::new(Text::from(lines)).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, area);
}

/// Builds the help-bar content for the current app state. The line count
/// this returns determines how many rows the caller reserves for the bar
/// (see `render_with_time`), so branches that don't need the full
/// key-hint list return just a single line rather than padding to a fixed height.
fn build_help_lines(app: &App) -> Vec<Line<'static>> {
    let bold = Style::default().add_modifier(Modifier::BOLD);

    if app.view == View::Clean {
        return build_clean_help_lines(app);
    }

    if let Some(line) = clean_status_line(app, bold) {
        return vec![line];
    }

    if app.view == View::Worktree {
        return build_worktree_help_lines(app, bold);
    }

    build_session_help_lines(app, bold)
}

/// Progress/summary line shown in place of the regular help bar while a
/// detached cleanup is in flight (or its "Done" summary hasn't been
/// dismissed yet). Returns `None` when there is nothing notable to show.
fn clean_status_line(app: &App, bold: Style) -> Option<Line<'static>> {
    let progress_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let progress = app.clean_progress.as_ref()?;
    Some(Line::from(vec![
        Span::raw("  "),
        Span::styled(progress.render_line(), progress_style),
        Span::raw("   "),
        Span::styled("q", bold),
        Span::raw(": quit  "),
        Span::styled("Tab", bold),
        Span::raw(": switch view"),
    ]))
}

/// Builds the collapsed `?: keys   <hint>   <hint>   ...` line shared by the
/// worktree and session views' default (non-expanded) help bar state.
fn build_compact_help_line(bold: Style, hints: &[(&str, &str)]) -> Vec<Line<'static>> {
    let mut spans = vec![
        Span::raw(" "),
        Span::styled("?", bold),
        Span::raw(": keys   "),
    ];
    for (i, (key, label)) in hints.iter().enumerate() {
        spans.push(Span::styled((*key).to_string(), bold));
        let sep = if i + 1 == hints.len() { "" } else { "   " };
        spans.push(Span::raw(format!(": {label}{sep}")));
    }
    vec![Line::from(spans)]
}

fn build_worktree_help_lines(app: &App, bold: Style) -> Vec<Line<'static>> {
    match &app.worktree_view.mode {
        WorktreeMode::Confirm {
            session_count,
            has_active,
            ..
        } => {
            let warn_color = if *has_active {
                Color::Red
            } else {
                Color::Yellow
            };
            let warn_style = Style::default().fg(warn_color).add_modifier(Modifier::BOLD);
            let prompt = if *has_active {
                format!(
                    "  WARNING: ACTIVE session — delete worktree and {session_count} session{}?",
                    if *session_count == 1 { "" } else { "s" }
                )
            } else if *session_count > 0 {
                format!(
                    "  Delete worktree and {session_count} session{}?",
                    if *session_count == 1 { "" } else { "s" }
                )
            } else {
                "  Delete worktree?".to_string()
            };
            vec![Line::from(vec![
                Span::styled(prompt, warn_style),
                Span::raw(" "),
                Span::styled("y", bold),
                Span::raw(": yes  "),
                Span::styled("n/Esc", bold),
                Span::raw(": cancel"),
            ])]
        }
        WorktreeMode::Normal if app.show_help => vec![Line::from(vec![
            Span::styled("  j/k", bold),
            Span::raw(": move  "),
            Span::styled("Enter/f", bold),
            Span::raw(": focus  "),
            Span::styled("d", bold),
            Span::raw(": delete  "),
            Span::styled("1-9", bold),
            Span::raw(": quick  "),
            Span::styled("Tab", bold),
            Span::raw(": switch view  "),
            Span::styled("q", bold),
            Span::raw(": quit"),
        ])],
        WorktreeMode::Normal => build_compact_help_line(
            bold,
            &[("Enter/f", "focus"), ("Tab", "switch view"), ("q", "quit")],
        ),
    }
}

fn build_session_help_lines(app: &App, bold: Style) -> Vec<Line<'static>> {
    match &app.mode {
        AppMode::Confirm {
            is_alive,
            worktree_cleanup,
            ..
        } => {
            let base = if *is_alive {
                "Stop and delete session"
            } else {
                "Delete session"
            };
            let suffix = if worktree_cleanup.is_some() {
                " (last in worktree; also deletes worktree, branch, tmux windows)"
            } else {
                ""
            };
            let prompt = format!("{base}{suffix}?");
            let warn_style = Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD);
            vec![Line::from(vec![
                Span::styled(format!("  {prompt} "), warn_style),
                Span::styled("y", bold),
                Span::raw(": yes  "),
                Span::styled("n/Esc", bold),
                Span::raw(": cancel"),
            ])]
        }
        AppMode::Search => vec![Line::from(vec![
            Span::styled("  C-n/C-p", bold),
            Span::raw(": move  "),
            Span::styled("Enter", bold),
            Span::raw(": focus  "),
            Span::styled("Esc", bold),
            Span::raw(": cancel"),
        ])],
        AppMode::Edit { .. } => vec![Line::from(vec![
            Span::styled("  Enter", bold),
            Span::raw(": save  "),
            Span::styled("Esc", bold),
            Span::raw(": cancel"),
        ])],
        AppMode::Normal if app.show_help && app.has_filter() => vec![
            Line::from(vec![
                Span::styled("  j/k", bold),
                Span::raw(": move  "),
                Span::styled("f", bold),
                Span::raw(": focus  "),
                Span::styled("r", bold),
                Span::raw(": resume  "),
                Span::styled("d", bold),
                Span::raw(": delete  "),
                Span::styled("/", bold),
                Span::raw(": edit  "),
                Span::styled("q", bold),
                Span::raw(": quit"),
            ]),
            Line::from(vec![
                Span::styled("  h/←", bold),
                Span::raw(": parent  "),
                Span::styled("→/l", bold),
                Span::raw(": drill down  "),
                Span::styled("C-r/w/s/p", bold),
                Span::raw(": filter  "),
                Span::styled("Esc", bold),
                Span::raw(": clear"),
            ]),
        ],
        AppMode::Normal if app.show_help => vec![
            Line::from(vec![
                Span::styled("  j/k", bold),
                Span::raw(": move  "),
                Span::styled("f", bold),
                Span::raw(": focus  "),
                Span::styled("r", bold),
                Span::raw(": resume  "),
                Span::styled("p", bold),
                Span::raw(": preview  "),
                Span::styled("d", bold),
                Span::raw(": delete  "),
                Span::styled("1-9", bold),
                Span::raw(": quick  "),
                Span::styled("/", bold),
                Span::raw(": search"),
            ]),
            Line::from(vec![
                Span::styled("  h/←", bold),
                Span::raw(": parent  "),
                Span::styled("→/l", bold),
                Span::raw(": drill down  "),
                Span::styled("C-r/w/s/p", bold),
                Span::raw(": filter  "),
                Span::styled("Tab", bold),
                Span::raw(": worktree view  "),
                Span::styled("q", bold),
                Span::raw(": quit"),
            ]),
        ],
        AppMode::Normal if app.has_filter() => build_compact_help_line(
            bold,
            &[
                ("/", "search"),
                ("Esc", "clear filter"),
                ("Tab", "worktree"),
                ("q", "quit"),
            ],
        ),
        AppMode::Normal => {
            build_compact_help_line(bold, &[("/", "search"), ("Tab", "worktree"), ("q", "quit")])
        }
    }
}

/// Extracts the clean-view's help/confirmation content. The bottom line is
/// the `Clean N worktree (M active excluded)? [y/N]` prompt; the line above
/// lists the basic key bindings. Always 2 lines — not gated by
/// `show_help`, since the delete-count prompt must always be visible.
fn build_clean_help_lines(app: &App) -> Vec<Line<'static>> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);
    let warn = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let (to_delete, kept_active) = app.clean_view.summary();
    let prompt = if to_delete == 0 {
        "  Nothing to clean. ".to_string()
    } else if kept_active > 0 {
        format!(
            "  Clean {to_delete} worktree{} ({kept_active} active excluded)? ",
            if to_delete == 1 { "" } else { "s" }
        )
    } else {
        format!(
            "  Clean {to_delete} worktree{}? ",
            if to_delete == 1 { "" } else { "s" }
        )
    };

    let help_line = Line::from(vec![
        Span::styled("  j/k", bold),
        Span::raw(": move  "),
        Span::styled("Enter", bold),
        Span::raw(": toggle / focus session  "),
        Span::styled("y", bold),
        Span::raw(": run  "),
        Span::styled("n/Esc/q", bold),
        Span::raw(": cancel"),
    ]);
    let prompt_line = if to_delete == 0 {
        Line::from(vec![
            Span::styled(prompt, dim),
            Span::styled("n/Esc/q", bold),
            Span::raw(": back"),
        ])
    } else {
        Line::from(vec![
            Span::styled(prompt, warn),
            Span::styled("y", bold),
            Span::raw(": run  "),
            Span::styled("N", bold),
            Span::raw(": cancel"),
        ])
    };
    vec![help_line, prompt_line]
}

/// Renders an error message at the bottom.
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
    use crate::commands::cc::tui::ui::test_support::{
        create_test_session, render_to_string, render_to_string_with, wt_row,
    };
    use crate::commands::cc::types::{Session, SessionStatus};
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
            " cc watch                                       1 needs you · 1 running · 2 idle"
        );
    }

    #[rstest]
    #[case::session_view_default(View::Session, false, vec![
        " ?: keys   /: search   Tab: worktree   q: quit".to_string(),
    ])]
    #[case::session_view_expanded(View::Session, true, vec![
        "  j/k: move  f: focus  r: resume  p: preview  d: delete  1-9: quick  /: search".to_string(),
        "  h/←: parent  →/l: drill down  C-r/w/s/p: filter  Tab: worktree view  q: quit".to_string(),
    ])]
    #[case::worktree_view_default(View::Worktree, false, vec![
        " ?: keys   Enter/f: focus   Tab: switch view   q: quit".to_string(),
    ])]
    #[case::worktree_view_expanded(View::Worktree, true, vec![
        "  j/k: move  Enter/f: focus  d: delete  1-9: quick  Tab: switch view  q: quit".to_string(),
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
