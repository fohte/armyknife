use crate::commands::cc::types::SessionStatus;
use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::commands::cc::tui::app::{App, AppMode, View};
use crate::commands::cc::tui::worktree_view::WorktreeMode;

use super::clean_list::render_clean_list;
use super::helpers::{count_statuses, truncate};
use super::session_list::render_session_list;
use super::worktree_list::render_worktree_list;

const HEADER_HEIGHT: u16 = 3;
const HELP_BAR_HEIGHT: u16 = 2;

/// Renders the entire UI.
pub fn render(frame: &mut Frame, app: &mut App) {
    render_with_time(frame, app, Utc::now());
}

pub(super) fn render_with_time(frame: &mut Frame, app: &mut App, now: DateTime<Utc>) {
    let area = frame.area();

    // The search bar is session-view only.
    let has_error = app.error_message.is_some();
    let is_search_mode = app.view == View::Session && app.mode == AppMode::Search;
    let has_text_filter = app.view == View::Session && !app.confirmed_query.is_empty();
    let show_search_bar = is_search_mode || has_text_filter;

    let layouts: Vec<Constraint> = match (show_search_bar, has_error) {
        (true, true) => vec![
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Length(1), // Search bar (at top)
            Constraint::Min(1),    // Session list
            Constraint::Length(HELP_BAR_HEIGHT),
            Constraint::Length(1), // Error
        ],
        (true, false) => vec![
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Length(1), // Search bar (at top)
            Constraint::Min(1),    // Session list
            Constraint::Length(HELP_BAR_HEIGHT),
        ],
        (false, true) => vec![
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Min(1), // Session list
            Constraint::Length(HELP_BAR_HEIGHT),
            Constraint::Length(1), // Error
        ],
        (false, false) => vec![
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Min(1), // Session list
            Constraint::Length(HELP_BAR_HEIGHT),
        ],
    };

    let areas = Layout::vertical(layouts).split(area);

    render_header(frame, areas[0], app);

    match (show_search_bar, has_error) {
        (true, true) => {
            render_search_input(frame, areas[1], app);
            render_main_list(frame, areas[2], app, now);
            render_help(frame, areas[3], app);
            render_error(frame, areas[4], app.error_message.as_deref().unwrap_or(""));
        }
        (true, false) => {
            render_search_input(frame, areas[1], app);
            render_main_list(frame, areas[2], app, now);
            render_help(frame, areas[3], app);
        }
        (false, true) => {
            render_main_list(frame, areas[1], app, now);
            render_help(frame, areas[2], app);
            render_error(frame, areas[3], app.error_message.as_deref().unwrap_or(""));
        }
        (false, false) => {
            render_main_list(frame, areas[1], app, now);
            render_help(frame, areas[2], app);
        }
    }
}

/// Returns the style for a status indicator, highlighted when it matches the active filter.
fn get_status_style(
    base_color: Color,
    status: SessionStatus,
    active_filter: Option<SessionStatus>,
) -> Style {
    let style = Style::default().fg(base_color);
    if active_filter == Some(status) {
        style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        style
    }
}

/// Renders the header with status counts.
/// When a status filter is active, the matching status is visually highlighted.
fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let (running, waiting, stopped, paused) = count_statuses(&app.sessions);
    let status_filter = app.status_filter;

    let running_style = get_status_style(Color::Green, SessionStatus::Running, status_filter);
    let waiting_style = get_status_style(Color::Yellow, SessionStatus::WaitingInput, status_filter);
    let stopped_style = get_status_style(Color::DarkGray, SessionStatus::Stopped, status_filter);
    let paused_style = get_status_style(Color::Indexed(245), SessionStatus::Paused, status_filter);

    let title = match app.view {
        View::Session => "  Claude Code Sessions",
        View::Worktree => "  Worktrees           ",
        View::Clean => "  Clean worktrees     ",
    };
    let status_line = Line::from(vec![
        Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("                       "),
        Span::styled(
            format!("{} {}", SessionStatus::Running.display_symbol(), running),
            running_style,
        ),
        Span::raw("  "),
        Span::styled(
            format!(
                "{} {}",
                SessionStatus::WaitingInput.display_symbol(),
                waiting
            ),
            waiting_style,
        ),
        Span::raw("  "),
        Span::styled(
            format!("{} {}", SessionStatus::Paused.display_symbol(), paused),
            paused_style,
        ),
        Span::raw("  "),
        Span::styled(
            format!("{} {}", SessionStatus::Stopped.display_symbol(), stopped),
            stopped_style,
        ),
    ]);

    let header = Paragraph::new(status_line).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    frame.render_widget(header, area);
}

/// Dispatch list rendering on the active view.
fn render_main_list(frame: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
    match app.view {
        View::Session => render_session_list(frame, area, app, now),
        View::Worktree => render_worktree_list(frame, area, app, now),
        View::Clean => render_clean_list(frame, area, app, now),
    }
}

/// Renders the help bar at the bottom.
fn render_help(frame: &mut Frame, area: Rect, app: &App) {
    let bold = Style::default().add_modifier(Modifier::BOLD);

    if app.view == View::Clean {
        render_clean_help(frame, area, app);
        return;
    }

    // While a detached clean is in flight (or a startup banner is
    // queued) the help bar's first line carries a progress / summary
    // notice instead of the usual key hints.
    if let Some(line) = clean_status_line(app) {
        let help_lines = vec![
            line,
            Line::from(vec![
                Span::styled("  q", bold),
                Span::raw(": quit  "),
                Span::styled("Tab", bold),
                Span::raw(": switch view"),
            ]),
        ];
        let help =
            Paragraph::new(Text::from(help_lines)).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(help, area);
        return;
    }

    // Worktree view has its own help line set.
    if app.view == View::Worktree {
        let help_lines: Vec<Line> = match &app.worktree_view.mode {
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
                vec![
                    Line::from(vec![
                        Span::styled(prompt, warn_style),
                        Span::raw(" "),
                        Span::styled("y", bold),
                        Span::raw(": yes  "),
                        Span::styled("n/Esc", bold),
                        Span::raw(": cancel"),
                    ]),
                    Line::from(""),
                ]
            }
            WorktreeMode::Normal => vec![
                Line::from(vec![
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
                ]),
                Line::from(""),
            ],
        };
        let help =
            Paragraph::new(Text::from(help_lines)).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(help, area);
        return;
    }

    let help_lines: Vec<Line> = match &app.mode {
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
            vec![
                Line::from(vec![
                    Span::styled(format!("  {prompt} "), warn_style),
                    Span::styled("y", bold),
                    Span::raw(": yes  "),
                    Span::styled("n/Esc", bold),
                    Span::raw(": cancel"),
                ]),
                Line::from(""),
            ]
        }
        AppMode::Search => vec![
            Line::from(vec![
                Span::styled("  C-n/C-p", bold),
                Span::raw(": move  "),
                Span::styled("Enter", bold),
                Span::raw(": focus  "),
                Span::styled("Esc", bold),
                Span::raw(": cancel"),
            ]),
            Line::from(""),
        ],
        AppMode::Normal if app.has_filter() => vec![
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
                Span::styled("  C-r/w/s/p", bold),
                Span::raw(": filter  "),
                Span::styled("Esc", bold),
                Span::raw(": clear"),
            ]),
        ],
        AppMode::Normal => vec![
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
                Span::styled("  C-r/w/s/p", bold),
                Span::raw(": filter  "),
                Span::styled("Tab", bold),
                Span::raw(": worktree view  "),
                Span::styled("q", bold),
                Span::raw(": quit"),
            ]),
        ],
    };

    let help = Paragraph::new(Text::from(help_lines)).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, area);
}

/// Status line shown in place of the regular help bar's top row when a
/// detached cleanup is in flight. Returns `None` when there is nothing
/// notable to display.
fn clean_status_line(app: &App) -> Option<Line<'static>> {
    let progress_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let progress = app.clean_progress.as_ref()?;
    Some(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(progress.render_line(), progress_style),
    ]))
}

/// Help / confirmation bar for the clean view. The bottom line is the
/// `Clean N worktree (M active excluded)? [y/N]` prompt; the line
/// above lists the basic key bindings.
fn render_clean_help(frame: &mut Frame, area: Rect, app: &App) {
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
    let help = Paragraph::new(Text::from(vec![help_line, prompt_line]))
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, area);
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
    let prefix = "  /";
    let cursor_str = if is_search_mode { "_" } else { "" };
    let count_width = count_str.len();
    let fixed_width = prefix.len() + cursor_str.len() + count_width + 2; // +2 for spacing
    let query_max_width = term_width.saturating_sub(fixed_width);

    // Truncate query if needed
    let display_query = truncate(query, query_max_width);

    // Calculate padding to right-align the count
    let content_width = prefix.len() + display_query.width() + cursor_str.len();
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
