use crate::commands::cc::types::{Session, SessionStatus};
use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{App, AppMode, View};
use super::clean_view::{CleanListEntry, CleanLoadState, CleanSection, PrFetchStatus};
use super::session_tree::{
    TreeEntry, build_line1_tree_prefix, build_line2_tree_prefix, build_parent_child_connector,
    build_separator_tree_prefix, build_session_tree,
};
use super::worktree_session_children::{create_session_child_list_item, format_relative_time};
use super::worktree_view::{WorktreeListEntry, WorktreeLoadState, WorktreeMode, WorktreeStatus};

/// Minimum width for session info on line 1
const MIN_SESSION_INFO_WIDTH: usize = 20;
/// Minimum width for tool/message content on line 2
const MIN_CONTENT_WIDTH: usize = 20;
/// Fixed width for time suffix: "  XXm ago" = ~12 chars
const LINE1_SUFFIX_WIDTH: usize = 12;

/// Number of distinct hue slots for repo label colors.
/// Using a prime number helps avoid systematic collisions with
/// common string patterns.
const REPO_LABEL_HUE_SLOTS: u64 = 31;

// Green-to-gray gradient for the "time ago" label
const TIME_AGO_BRIGHT_GREEN: Color = Color::Indexed(82);
const TIME_AGO_GREEN: Color = Color::Indexed(78);
const TIME_AGO_DARK_GREEN: Color = Color::Indexed(72);
const TIME_AGO_DARKER_GREEN: Color = Color::Indexed(65);
const TIME_AGO_LIGHT_GRAY: Color = Color::Indexed(245);
const TIME_AGO_DARK_GRAY: Color = Color::Indexed(241);
const HEADER_HEIGHT: u16 = 3;
const HELP_BAR_HEIGHT: u16 = 2;

/// Renders the entire UI.
pub fn render(frame: &mut Frame, app: &mut App) {
    render_with_time(frame, app, Utc::now());
}

fn render_with_time(frame: &mut Frame, app: &mut App, now: DateTime<Utc>) {
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

/// Renders the clean view: To delete / Kept sections, repo group
/// headers under each section, one row per worktree, then nested
/// session rows under each worktree.
fn render_clean_list(frame: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
    let term_width = area.width as usize;
    let state = app.clean_view.state.clone();
    match state {
        CleanLoadState::LoadingPr => {
            // No initial worktree snapshot yet — the only thing to show
            // is the pending state. Once the snapshot arrives the view
            // re-renders as Ready.
            let p = Paragraph::new("  Loading worktrees...")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(p, area);
            return;
        }
        CleanLoadState::Failed(err) => {
            let line = Line::from(vec![
                Span::styled(
                    "  Failed to load PR status: ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled(err, Style::default().fg(Color::Red)),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            return;
        }
        CleanLoadState::Ready(_) => {}
    }

    let entries = app.clean_view.list_entries();
    if entries.is_empty() {
        let p =
            Paragraph::new("  No worktrees to clean.").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, area);
        return;
    }

    // Reserve one line at the top for the PR-fetch status banner when
    // the fetch is either still running or has failed.
    let banner = pr_fetch_banner(&app.clean_view.pr_fetch);
    let (banner_area, list_area) = if banner.is_some() {
        let chunks = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, area)
    };
    if let (Some(area), Some(p)) = (banner_area, banner) {
        frame.render_widget(p, area);
    }

    let items: Vec<ListItem> = entries
        .iter()
        .map(|e| create_clean_list_item(e, term_width, now))
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">");

    frame.render_stateful_widget(list, list_area, &mut app.clean_view.list_state);
}

fn pr_fetch_banner(status: &PrFetchStatus) -> Option<Paragraph<'static>> {
    match status {
        PrFetchStatus::Loading => Some(
            Paragraph::new("  Fetching PR status... (toggle disabled)")
                .style(Style::default().fg(Color::DarkGray)),
        ),
        PrFetchStatus::Failed(err) => {
            let line = Line::from(vec![
                Span::styled(
                    "  PR fetch failed: ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled(err.clone(), Style::default().fg(Color::Red)),
            ]);
            Some(Paragraph::new(line))
        }
        PrFetchStatus::Done => None,
    }
}

fn create_clean_list_item(
    entry: &CleanListEntry,
    term_width: usize,
    now: DateTime<Utc>,
) -> ListItem<'static> {
    let dim_style = Style::default().fg(Color::DarkGray);
    let bold = Style::default().add_modifier(Modifier::BOLD);

    match entry {
        CleanListEntry::Session(child) => create_session_child_list_item(child, now),
        CleanListEntry::SectionHeader { section, count } => {
            let label = match section {
                CleanSection::ToDelete => format!("── To delete ({count}) "),
                CleanSection::Kept => format!("── Kept ({count}) "),
            };
            let color = match section {
                CleanSection::ToDelete => Color::Red,
                CleanSection::Kept => Color::Green,
            };
            // Pad with em-dashes to fill the row visually.
            let pad_width = term_width.saturating_sub(label.width()).min(80);
            let mut padded = label.clone();
            for _ in 0..pad_width {
                padded.push('─');
            }
            let line = Line::from(vec![Span::styled(
                padded,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )]);
            ListItem::new(vec![line])
        }
        CleanListEntry::RepoHeader(name) => {
            let line = Line::from(vec![Span::styled(
                format!("▼ {name}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]);
            ListItem::new(vec![line])
        }
        CleanListEntry::Row(row) => {
            let (symbol, color) = if row.has_active {
                ("◐", Color::Yellow)
            } else if row.session_count > 0 {
                ("●", Color::Green)
            } else {
                ("◌", Color::DarkGray)
            };
            let repo_color = repo_label_color(&row.repo);
            let bar = Span::styled("▎", Style::default().fg(repo_color));

            let primary = if row.repo == row.name || row.branch.is_empty() {
                row.repo.clone()
            } else {
                format!("{} {}", row.repo, row.branch)
            };
            let label = format!("[{}]", row.status_label);
            let primary_truncated =
                truncate(&primary, term_width.saturating_sub(8 + label.width() + 2));

            let line1 = Line::from(vec![
                Span::raw("  "),
                Span::styled(symbol.to_string(), Style::default().fg(color)),
                Span::raw(" "),
                bar.clone(),
                Span::raw(" "),
                Span::styled(primary_truncated, bold),
                Span::raw("  "),
                Span::styled(label, dim_style),
            ]);

            let detail = format!(
                "{} session{} · {}",
                row.session_count,
                if row.session_count == 1 { "" } else { "s" },
                row.path.display()
            );
            let detail = truncate(&detail, term_width.saturating_sub(8));
            let line2 = Line::from(vec![
                Span::raw("    "),
                bar,
                Span::raw(" "),
                Span::styled(detail, dim_style),
            ]);

            ListItem::new(vec![line1, line2])
        }
    }
}

/// Renders the worktree list, grouped by repo.
fn render_worktree_list(frame: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
    let term_width = area.width as usize;
    let state = app.worktree_view.state.clone();
    match state {
        WorktreeLoadState::Loading => {
            let p = Paragraph::new("  Loading worktrees...")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(p, area);
            return;
        }
        WorktreeLoadState::Failed(err) => {
            let line = Line::from(vec![
                Span::styled(
                    "  Failed to load worktrees: ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled(err, Style::default().fg(Color::Red)),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            return;
        }
        WorktreeLoadState::Loaded(_) => {}
    }

    let entries = app.worktree_view.list_entries();
    if entries.is_empty() {
        let p = Paragraph::new("  No linked worktrees discovered.")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, area);
        return;
    }

    let items: Vec<ListItem> = entries
        .iter()
        .map(|e| create_worktree_list_item(e, term_width, now))
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">");

    frame.render_stateful_widget(list, area, &mut app.worktree_view.list_state);
}

/// Returns the symbol + color used for one worktree row.
fn worktree_status_glyph(status: WorktreeStatus) -> (&'static str, Color) {
    match status {
        WorktreeStatus::Orphan => ("◌", Color::DarkGray),
        WorktreeStatus::Active => ("◐", Color::Yellow),
        WorktreeStatus::Idle => ("●", Color::Green),
    }
}

fn create_worktree_list_item(
    entry: &WorktreeListEntry,
    term_width: usize,
    now: DateTime<Utc>,
) -> ListItem<'static> {
    let dim_style = Style::default().fg(Color::DarkGray);
    let bold = Style::default().add_modifier(Modifier::BOLD);

    match entry {
        WorktreeListEntry::Session(child) => create_session_child_list_item(child, now),
        WorktreeListEntry::RepoHeader(name) => {
            let line = Line::from(vec![Span::styled(
                format!("▼ {}", name),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]);
            ListItem::new(vec![line])
        }
        WorktreeListEntry::Worktree(row) => {
            let (symbol, color) = worktree_status_glyph(row.status());
            let repo_color = repo_label_color(&row.repo);
            let bar = Span::styled("▎", Style::default().fg(repo_color));

            // Line 1: "  {status} ▎ {repo} {branch}"
            let primary = if row.repo == row.name || row.branch.is_empty() {
                row.repo.clone()
            } else {
                format!("{} {}", row.repo, row.branch)
            };
            let primary = truncate(&primary, term_width.saturating_sub(8));
            let line1 = Line::from(vec![
                Span::raw("  "),
                Span::styled(symbol.to_string(), Style::default().fg(color)),
                Span::raw(" "),
                bar.clone(),
                Span::raw(" "),
                Span::styled(primary, bold),
            ]);

            // Line 2: "    ▎ {n} sessions · {path}"
            // Leading width = indent (2) + status glyph width (1) + space (1) = 4
            // so the bar lines up under the bar on line 1.
            let detail = format!(
                "{} session{} · {}",
                row.session_count,
                if row.session_count == 1 { "" } else { "s" },
                row.path.display()
            );
            let detail = truncate(&detail, term_width.saturating_sub(8));
            let line2 = Line::from(vec![
                Span::raw("    "),
                bar,
                Span::raw(" "),
                Span::styled(detail, dim_style),
            ]);

            ListItem::new(vec![line1, line2])
        }
    }
}

/// Renders the session list with tree view.
fn render_session_list(frame: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
    let filtered_sessions: Vec<&Session> = app.filtered_sessions();

    if filtered_sessions.is_empty() {
        let message = if app.mode == AppMode::Search {
            format!("  No sessions match \"{}\"", app.search_query)
        } else if app.has_filter() {
            let mut parts = Vec::new();
            if let Some(status) = app.status_filter {
                parts.push(format!("status:{}", status.display_name()));
            }
            if !app.confirmed_query.is_empty() {
                parts.push(format!("\"{}\"", app.confirmed_query));
            }
            format!("  No sessions match {}", parts.join(" + "))
        } else {
            "  No active Claude Code sessions.".to_string()
        };
        let empty_message = Paragraph::new(message).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty_message, area);
        return;
    }

    let term_width = area.width as usize;

    // Determine the active search query for highlighting.
    // Clone to avoid borrowing app across the mutable cache update.
    let query = if app.mode == AppMode::Search {
        app.search_query.clone()
    } else {
        app.confirmed_query.clone()
    };

    // Build tree structure from sessions
    let tree_entries = build_session_tree(&filtered_sessions);

    // Collect tree-ordered session IDs and build list items, then drop
    // tree_entries (which borrows filtered_sessions/app) before mutating app.
    let mut tree_session_ids: Vec<String> = Vec::with_capacity(tree_entries.len());
    let items: Vec<ListItem> = tree_entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            tree_session_ids.push(entry.session.session_id.clone());
            let next_entry = tree_entries.get(i + 1);
            let cached_title = app.get_cached_title(&entry.session.session_id);
            let (repo_name, worktree_name) = app
                .get_cached_worktree_labels(&entry.session.cwd)
                .unwrap_or(("", ""));
            create_tree_session_item(
                entry,
                next_entry,
                cached_title,
                now,
                term_width,
                &query,
                repo_name,
                worktree_name,
            )
        })
        .collect();
    drop(tree_entries);
    drop(filtered_sessions);

    // Sync tree-ordered indices so selection maps to the correct session
    let tree_id_refs: Vec<&str> = tree_session_ids.iter().map(|s| s.as_str()).collect();
    app.update_tree_order(&tree_id_refs);

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">");

    frame.render_stateful_widget(list, area, &mut app.list_state);
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

/// Creates a list item for a session within a tree view.
///
/// Each session renders as 2 lines:
/// - Line 1: [tree_prefix] status_symbol ▎ session_info  label  time_ago
/// - Line 2: [tree_prefix_continuation]  ▎ current_tool or last_message
///
/// Plus separator lines between tree entries (empty lines with connectors).
#[expect(
    clippy::too_many_arguments,
    reason = "Tree-rendering needs entry + neighbour + caches + repo/worktree labels in one call"
)]
fn create_tree_session_item(
    entry: &TreeEntry,
    next_entry: Option<&TreeEntry>,
    cached_title: Option<&str>,
    now: DateTime<Utc>,
    term_width: usize,
    query: &str,
    repo_name: &str,
    worktree_name: &str,
) -> ListItem<'static> {
    let session = entry.session;
    let status_symbol = session.display_symbol();
    let s_color = status_color(session.status);
    let session_info = get_session_info(session, repo_name, worktree_name);
    let label = cached_title
        .map(String::from)
        .unwrap_or_else(|| get_title_display_name_fallback(session));
    let time_ago = format_relative_time(session.updated_at, now);

    let repo_color = repo_label_color(repo_name);
    let bar = Span::styled("▎", Style::default().fg(repo_color));
    let dim_style = Style::default().fg(Color::DarkGray);

    let line1_tree_prefix = build_line1_tree_prefix(entry);
    let line2_tree_prefix = build_line2_tree_prefix(entry);

    // Display width of the tree prefix + status symbol + bar + spacing
    // Line 1: "{tree_prefix}{status} ▎ {session_info}  {label}  {time_ago}"
    let line1_prefix_display_width =
        line1_tree_prefix.width() + status_symbol.width() + " ▎ ".width();
    let line1_fixed_width = line1_prefix_display_width + LINE1_SUFFIX_WIDTH;

    let session_info_width = if term_width > line1_fixed_width + MIN_SESSION_INFO_WIDTH {
        term_width - line1_fixed_width
    } else {
        MIN_SESSION_INFO_WIDTH
    };

    // Line 2: "{tree_prefix}  ▎ {content}"
    let line2_prefix_display_width = line2_tree_prefix.width() + "  ▎ ".width();
    let content_width = if term_width > line2_prefix_display_width + MIN_CONTENT_WIDTH {
        term_width - line2_prefix_display_width
    } else {
        MIN_CONTENT_WIDTH
    };

    let time_ago_fg = time_ago_color(session.updated_at, now);

    // Build combined info string: "session_info  label"
    // Skip label if it duplicates session_info (common when no explicit label is set)
    let combined_info = if label.is_empty() || label == session_info {
        session_info.clone()
    } else {
        format!("{}  {}", session_info, label)
    };
    let truncated_info = truncate(&combined_info, session_info_width);
    let is_paused = session.status == SessionStatus::Paused;
    let paused_style = Style::default().fg(Color::Indexed(245));
    let info_style = if is_paused {
        paused_style
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };

    // Line 1: [tree_prefix] status ▎ session_info  label  time_ago
    let mut line1_spans = Vec::new();
    if !line1_tree_prefix.is_empty() {
        line1_spans.push(Span::styled(line1_tree_prefix, dim_style));
    }
    line1_spans.push(Span::styled(status_symbol, Style::default().fg(s_color)));
    line1_spans.push(Span::raw(" "));
    line1_spans.push(bar.clone());
    line1_spans.push(Span::raw(" "));
    line1_spans.extend(highlight_matches(&truncated_info, query, info_style));
    line1_spans.push(Span::raw("  "));
    let time_style = if is_paused {
        paused_style
    } else {
        Style::default().fg(time_ago_fg)
    };
    line1_spans.push(Span::styled(time_ago, time_style));
    let line1 = Line::from(line1_spans);

    // Line 2: [tree_prefix_continuation]  ▎ current_tool or last_message
    let line2_content = session
        .current_tool
        .as_deref()
        .or(session.last_message.as_deref())
        .unwrap_or("");
    let truncated_content = truncate(line2_content, content_width);
    let content_style = if is_paused {
        paused_style
    } else {
        Style::default().add_modifier(Modifier::DIM)
    };
    let mut line2_spans = Vec::new();
    if !line2_tree_prefix.is_empty() {
        line2_spans.push(Span::styled(line2_tree_prefix, dim_style));
    }
    line2_spans.push(Span::raw("  "));
    line2_spans.push(bar.clone());
    line2_spans.push(Span::raw(" "));
    line2_spans.extend(highlight_matches(&truncated_content, query, content_style));
    let line2 = Line::from(line2_spans);

    // Build separator lines
    let mut lines = vec![line1, line2];

    // Add separator line(s) between this entry and the next
    if let Some(next) = next_entry {
        if entry.has_children {
            // This node has children, next is its first child:
            // show "│" connector between parent and children block
            let connector = build_parent_child_connector(entry);
            if !connector.is_empty() {
                lines.push(Line::from(Span::styled(connector, dim_style)));
            } else {
                lines.push(Line::from(""));
            }
        } else if next.depth < entry.depth {
            // Going back up the tree: show blank line
            lines.push(Line::from(""));
        } else if next.depth == entry.depth && next.depth == 0 {
            // Between root-level tree groups: blank line separator
            lines.push(Line::from(""));
        } else {
            // Between siblings: show separator with pipe
            let sep = build_separator_tree_prefix(entry);
            if !sep.is_empty() {
                lines.push(Line::from(Span::styled(sep, dim_style)));
            } else {
                lines.push(Line::from(""));
            }
        }
    }
    // No separator after the very last entry

    ListItem::new(lines)
}

/// Returns a deterministic color for a repository name.
/// Uses FNV-1a hash for good distribution, then maps to a hue on the
/// HSL color wheel with fixed saturation and lightness for terminal
/// readability.
fn repo_label_color(repo_name: &str) -> Color {
    // FNV-1a hash for better distribution than simple multiply-add
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001B3;
    // FNV-1a: XOR first, then multiply
    let hash = repo_name.bytes().fold(FNV_OFFSET, |acc, b| {
        (acc ^ (b as u64)).wrapping_mul(FNV_PRIME)
    });

    let slot = (hash % REPO_LABEL_HUE_SLOTS) as f64;
    let hue = (slot / REPO_LABEL_HUE_SLOTS as f64) * 360.0;
    let (r, g, b) = hsl_to_rgb(hue, 0.65, 0.45);
    Color::Rgb(r, g, b)
}

/// Converts HSL color to RGB (each component 0-255).
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

/// Returns the color for a session status icon.
fn status_color(status: SessionStatus) -> Color {
    match status {
        SessionStatus::Running => Color::Green,
        SessionStatus::WaitingInput => Color::Yellow,
        // Paused gets a lighter gray than Stopped so the ⏸ icon stays
        // readable; the text itself is dimmed separately.
        SessionStatus::Paused => Color::Indexed(245),
        SessionStatus::Stopped | SessionStatus::Ended => Color::DarkGray,
    }
}

/// Returns a color for the "time ago" label based on elapsed time since last update.
///
/// Uses a green-to-gray gradient: bright green for recent activity, fading
/// through darker greens, then to gray for stale sessions.
fn time_ago_color(updated_at: DateTime<Utc>, now: DateTime<Utc>) -> Color {
    let seconds = now.signed_duration_since(updated_at).num_seconds().max(0);
    let minutes = seconds / 60;

    match minutes {
        // < 1 minute: bright green
        0 => TIME_AGO_BRIGHT_GREEN,
        // 1-5 minutes: green
        1..=5 => TIME_AGO_GREEN,
        // 6-30 minutes: slightly darker green
        6..=30 => TIME_AGO_DARK_GREEN,
        // 31-60 minutes: dark green
        31..=60 => TIME_AGO_DARKER_GREEN,
        // 1-6 hours: light gray
        61..=360 => TIME_AGO_LIGHT_GRAY,
        // 6+ hours: darker gray (floor)
        _ => TIME_AGO_DARK_GRAY,
    }
}

/// Counts sessions by status.
fn count_statuses(sessions: &[Session]) -> (usize, usize, usize, usize) {
    let mut running = 0;
    let mut waiting = 0;
    let mut stopped = 0;
    let mut paused = 0;

    for session in sessions {
        match session.status {
            SessionStatus::Running => running += 1,
            SessionStatus::WaitingInput => waiting += 1,
            SessionStatus::Paused => paused += 1,
            SessionStatus::Stopped | SessionStatus::Ended => stopped += 1,
        }
    }

    (running, waiting, stopped, paused)
}

/// Gets the title display name for a session without external file I/O.
/// Used as fallback when title is not in cache.
/// All outputs are sanitized to strip ANSI escape sequences.
fn get_title_display_name_fallback(session: &Session) -> String {
    use crate::commands::cc::claude_sessions;

    if let Some(ref label) = session.label {
        return claude_sessions::normalize_title(label);
    }

    let raw_title = session
        .cwd
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from)
        .unwrap_or_else(|| session.cwd.display().to_string());
    claude_sessions::normalize_title(&raw_title)
}

/// Gets the session info field rendered next to the repo bar.
/// Format: `<repo> <worktree-name>`. When neither label is available yet
/// (first frame before the cache is populated), falls back to the cwd
/// basename so the row stays within reasonable width.
/// All outputs are sanitized to strip ANSI escape sequences.
fn get_session_info(session: &Session, repo: &str, worktree_name: &str) -> String {
    use crate::commands::cc::claude_sessions;

    let raw = if !repo.is_empty() && !worktree_name.is_empty() {
        if repo == worktree_name {
            repo.to_string()
        } else {
            format!("{repo} {worktree_name}")
        }
    } else if !repo.is_empty() {
        repo.to_string()
    } else if !worktree_name.is_empty() {
        worktree_name.to_string()
    } else {
        session
            .cwd
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| session.cwd.display().to_string())
    };
    claude_sessions::normalize_title(&raw)
}

/// Splits text into spans, highlighting portions that match any of the search words.
///
/// Uses case-insensitive matching consistent with the existing search logic.
/// When multiple words produce overlapping match ranges, they are merged.
/// Highlighted spans receive `fg(Color::Yellow)` and `BOLD` on top of `base_style`.
fn highlight_matches<'a>(text: &str, query: &str, base_style: Style) -> Vec<Span<'a>> {
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() || text.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    // Build a mapping from lowercased byte offsets to original byte offsets.
    // to_lowercase() can change byte length (e.g. 'İ' -> "i\u{307}"),
    // so we must map match positions in the lowercased string back to the original.
    let mut text_lower = String::new();
    let mut lower_to_orig: Vec<usize> = Vec::new();
    for (orig_offset, ch) in text.char_indices() {
        for lower_ch in ch.to_lowercase() {
            let lower_start = text_lower.len();
            text_lower.push(lower_ch);
            // Map each byte of the lowercased char to the original char's byte offset
            for _ in lower_start..text_lower.len() {
                lower_to_orig.push(orig_offset);
            }
        }
    }
    // Sentinel: map end-of-lowered-string to end-of-original-string
    lower_to_orig.push(text.len());

    // Collect all match ranges (byte offsets in the original string)
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for word in &words {
        let word_lower = word.to_lowercase();
        let mut start = 0;
        while let Some(pos) = text_lower[start..].find(&word_lower) {
            let lower_start = start + pos;
            let lower_end = lower_start + word_lower.len();
            let orig_start = lower_to_orig[lower_start];
            let mut orig_end = lower_to_orig[lower_end];
            // When lowercasing expands bytes (e.g. İ -> i\u{307}), a match
            // within the expansion maps start and end to the same original
            // offset. Extend to cover the full original character.
            if orig_end <= orig_start {
                orig_end = text[orig_start..]
                    .chars()
                    .next()
                    .map_or(text.len(), |c| orig_start + c.len_utf8());
            }
            ranges.push((orig_start, orig_end));
            // Advance by one character (not one byte) to stay on a char boundary
            start = lower_start
                + text_lower[lower_start..]
                    .chars()
                    .next()
                    .map_or(1, |c| c.len_utf8());
        }
    }

    if ranges.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    // Sort by start position, then by end position descending to prefer longer matches
    ranges.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    // Merge overlapping ranges
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.0 <= last.1
        {
            last.1 = last.1.max(range.1);
            continue;
        }
        merged.push(range);
    }

    // Build spans from merged ranges
    let highlight_style = base_style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    let mut cursor = 0;

    for (start, end) in merged {
        if cursor < start {
            spans.push(Span::styled(text[cursor..start].to_string(), base_style));
        }
        spans.push(Span::styled(text[start..end].to_string(), highlight_style));
        cursor = end;
    }

    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_string(), base_style));
    }

    spans
}

/// Truncates a string to fit within the specified display width.
/// Uses unicode display width for proper handling of CJK characters.
fn truncate(s: &str, max_width: usize) -> String {
    let display_width = s.width();
    if display_width <= max_width {
        s.to_string()
    } else if max_width < 3 {
        truncate_to_width(s, max_width)
    } else {
        let truncated = truncate_to_width(s, max_width - 3);
        format!("{}...", truncated)
    }
}

/// Truncates a string to fit within the specified display width.
fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut current_width = 0;

    for c in s.chars() {
        let char_width = c.width().unwrap_or(0);
        if current_width + char_width > max_width {
            break;
        }
        result.push(c);
        current_width += char_width;
    }

    result
}

/// Renders the entire UI to a TestBackend for testing.
/// Returns the rendered output as a string.
#[cfg(test)]
fn render_to_string(
    sessions: &[Session],
    selected_index: Option<usize>,
    now: DateTime<Utc>,
    width: u16,
    height: u16,
) -> String {
    render_to_string_with(sessions, selected_index, now, width, height, |_| {})
}

/// Same as `render_to_string`, but lets the caller mutate the `App`
/// between construction and render — useful to flip into the worktree
/// view, inject worktree rows, etc.
#[cfg(test)]
fn render_to_string_with<F>(
    sessions: &[Session],
    selected_index: Option<usize>,
    now: DateTime<Utc>,
    width: u16,
    height: u16,
    setup: F,
) -> String
where
    F: FnOnce(&mut App),
{
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut app = App::with_sessions(sessions.to_vec());
    app.list_state.select(selected_index);
    setup(&mut app);

    terminal
        .draw(|frame| {
            render_with_time(frame, &mut app, now);
        })
        .unwrap();

    // Convert buffer to string
    let backend = terminal.backend();
    let buffer = backend.buffer();
    let mut output = String::new();

    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            output.push_str(cell.symbol());
        }
        // Trim trailing whitespace and add newline
        let trimmed = output.trim_end_matches(' ');
        output.truncate(trimmed.len());
        output.push('\n');
    }

    // Remove trailing newline
    if output.ends_with('\n') {
        output.pop();
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::TmuxInfo;
    use chrono::Duration;
    use indoc::indoc;
    use rstest::rstest;
    use std::path::PathBuf;

    fn create_test_session(id: &str) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/home/user/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            pending_agent_task_ids: std::collections::BTreeSet::new(),
            read_at: None,
            sweep_signaled: false,
        }
    }

    #[test]
    fn test_count_statuses() {
        let sessions = vec![
            {
                let mut s = create_test_session("1");
                s.status = SessionStatus::Running;
                s
            },
            {
                let mut s = create_test_session("2");
                s.status = SessionStatus::Running;
                s
            },
            {
                let mut s = create_test_session("3");
                s.status = SessionStatus::WaitingInput;
                s
            },
            {
                let mut s = create_test_session("4");
                s.status = SessionStatus::Stopped;
                s
            },
        ];

        let (running, waiting, stopped, paused) = count_statuses(&sessions);
        assert_eq!(running, 2);
        assert_eq!(waiting, 1);
        assert_eq!(stopped, 1);
        assert_eq!(paused, 0);
    }

    #[test]
    fn test_count_statuses_with_paused() {
        let sessions = vec![
            {
                let mut s = create_test_session("1");
                s.status = SessionStatus::Running;
                s
            },
            {
                let mut s = create_test_session("2");
                s.status = SessionStatus::Paused;
                s
            },
            {
                let mut s = create_test_session("3");
                s.status = SessionStatus::Paused;
                s
            },
        ];

        let (running, waiting, stopped, paused) = count_statuses(&sessions);
        assert_eq!(running, 1);
        assert_eq!(waiting, 0);
        assert_eq!(stopped, 0);
        assert_eq!(paused, 2);
    }

    #[test]
    fn test_get_title_display_name_fallback_ignores_tmux() {
        // Tmux session:window is no longer used as a label fallback; the
        // row already shows `<repo> <worktree-name>` instead. Without an
        // explicit label, the fallback is the cwd basename.
        let mut session = create_test_session("test");
        session.tmux_info = Some(TmuxInfo {
            session_name: "dev".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        assert_eq!(get_title_display_name_fallback(&session), "project");
    }

    #[test]
    fn test_get_title_display_name_fallback_to_cwd() {
        // When cache is not available and no tmux, falls back to cwd
        let session = create_test_session("test");
        assert_eq!(get_title_display_name_fallback(&session), "project");
    }

    #[rstest]
    #[case::repo_only("armyknife", "", "armyknife")]
    #[case::worktree_only("", "feat-x", "feat-x")]
    #[case::repo_and_worktree("armyknife", "feat-x", "armyknife feat-x")]
    #[case::dedup_when_equal("docs", "docs", "docs")]
    #[case::empty_falls_back_to_cwd_basename("", "", "project")]
    fn test_get_session_info_formats_repo_and_worktree(
        #[case] repo: &str,
        #[case] worktree: &str,
        #[case] expected: &str,
    ) {
        let session = create_test_session("test");
        assert_eq!(get_session_info(&session, repo, worktree), expected);
    }

    #[rstest]
    #[case::just_now(0, "just now")]
    #[case::one_minute(60, "1m ago")]
    #[case::two_hours(7200, "2h ago")]
    #[case::one_day(86400, "1d ago")]
    fn test_format_relative_time(#[case] seconds_ago: i64, #[case] expected: &str) {
        let now = Utc::now();
        let dt = now - Duration::seconds(seconds_ago);
        assert_eq!(format_relative_time(dt, now), expected);
    }

    #[rstest]
    #[case::short("hello", 10, "hello")]
    #[case::exact("hello", 5, "hello")]
    #[case::truncate("hello world", 8, "hello...")]
    #[case::cjk_short("日本語", 10, "日本語")]
    #[case::cjk_exact("日本語", 6, "日本語")]
    #[case::cjk_truncate("日本語テスト", 8, "日本...")]
    fn test_truncate(#[case] input: &str, #[case] max_width: usize, #[case] expected: &str) {
        assert_eq!(truncate(input, max_width), expected);
    }

    #[test]
    fn test_status_color() {
        assert_eq!(status_color(SessionStatus::Running), Color::Green);
        assert_eq!(status_color(SessionStatus::WaitingInput), Color::Yellow);
        assert_eq!(status_color(SessionStatus::Stopped), Color::DarkGray);
        assert_eq!(status_color(SessionStatus::Paused), Color::Indexed(245));
    }

    #[rstest]
    #[case::just_now(0, Color::Indexed(82))]
    #[case::thirty_seconds(30, Color::Indexed(82))]
    #[case::almost_one_minute(59, Color::Indexed(82))]
    #[case::one_minute(60, Color::Indexed(78))]
    #[case::three_minutes(180, Color::Indexed(78))]
    #[case::five_minutes(300, Color::Indexed(78))]
    #[case::six_minutes(360, Color::Indexed(72))]
    #[case::fifteen_minutes(900, Color::Indexed(72))]
    #[case::thirty_minutes(1800, Color::Indexed(72))]
    #[case::thirty_one_minutes(1860, Color::Indexed(65))]
    #[case::forty_five_minutes(2700, Color::Indexed(65))]
    #[case::one_hour(3600, Color::Indexed(65))]
    #[case::two_hours(7200, Color::Indexed(245))]
    #[case::five_hours(18000, Color::Indexed(245))]
    #[case::six_hours(21600, Color::Indexed(245))]
    #[case::seven_hours(25200, Color::Indexed(241))]
    #[case::one_day(86400, Color::Indexed(241))]
    fn test_time_ago_color(#[case] seconds_ago: i64, #[case] expected: Color) {
        let now = Utc::now();
        let updated_at = now - Duration::seconds(seconds_ago);
        assert_eq!(time_ago_color(updated_at, now), expected);
    }

    // =========================================================================
    // Full-screen integration tests using TestBackend (tree view)
    // =========================================================================

    #[test]
    fn test_render_flat_sessions_tree_view() {
        let now = Utc::now();

        let mut session1 = create_test_session("s1");
        session1.updated_at = now;
        session1.tmux_info = Some(TmuxInfo {
            session_name: "webapp".to_string(),
            window_name: "dev".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        session1.status = SessionStatus::Running;

        let mut session2 = create_test_session("s2");
        session2.updated_at = now - Duration::minutes(5);
        session2.tmux_info = Some(TmuxInfo {
            session_name: "api".to_string(),
            window_name: "test".to_string(),
            window_index: 1,
            pane_id: "%1".to_string(),
        });
        session2.status = SessionStatus::WaitingInput;

        let sessions = vec![session1, session2];
        let output = render_to_string(&sessions, Some(0), now, 80, 13);

        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 1  ◐ 1  ⏸ 0  ○ 0               │
            └──────────────────────────────────────────────────────────────────────────────┘
            >● ▎ project  just now
               ▎

             ◐ ▎ project  5m ago
               ▎



              j/k: move  f: focus  r: resume  p: preview  d: delete  1-9: quick  /: search
              C-r/w/s/p: filter  Tab: worktree view  q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_full_screen_empty_sessions() {
        let now = Utc::now();
        let sessions: Vec<Session> = vec![];
        let output = render_to_string(&sessions, None, now, 80, 9);

        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 0  ◐ 0  ⏸ 0  ○ 0               │
            └──────────────────────────────────────────────────────────────────────────────┘
              No active Claude Code sessions.



              j/k: move  f: focus  r: resume  p: preview  d: delete  1-9: quick  /: search
              C-r/w/s/p: filter  Tab: worktree view  q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_session_with_last_message() {
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.tmux_info = Some(TmuxInfo {
            session_name: "webapp".to_string(),
            window_name: "dev".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        session.status = SessionStatus::Running;
        session.last_message = Some("I've updated the code as requested.".to_string());

        let sessions = vec![session];
        let output = render_to_string(&sessions, Some(0), now, 80, 9);

        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 1  ◐ 0  ⏸ 0  ○ 0               │
            └──────────────────────────────────────────────────────────────────────────────┘
            >● ▎ project  just now
               ▎ I've updated the code as requested.


              j/k: move  f: focus  r: resume  p: preview  d: delete  1-9: quick  /: search
              C-r/w/s/p: filter  Tab: worktree view  q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_session_without_last_message() {
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.tmux_info = Some(TmuxInfo {
            session_name: "webapp".to_string(),
            window_name: "dev".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        session.status = SessionStatus::Running;

        let sessions = vec![session];
        let output = render_to_string(&sessions, Some(0), now, 80, 9);

        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 1  ◐ 0  ⏸ 0  ○ 0               │
            └──────────────────────────────────────────────────────────────────────────────┘
            >● ▎ project  just now
               ▎


              j/k: move  f: focus  r: resume  p: preview  d: delete  1-9: quick  /: search
              C-r/w/s/p: filter  Tab: worktree view  q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_session_with_label_different_from_info() {
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.cwd = PathBuf::from("/home/user/docs");
        session.status = SessionStatus::Stopped;
        // No tmux, so session_info = "/home/user/docs", fallback title = "docs"
        // These differ, so label "docs" should appear

        let sessions = vec![session];
        let output = render_to_string(&sessions, Some(0), now, 80, 9);

        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 0  ◐ 0  ⏸ 0  ○ 1               │
            └──────────────────────────────────────────────────────────────────────────────┘
            >✱ ▎ docs  just now
               ▎


              j/k: move  f: focus  r: resume  p: preview  d: delete  1-9: quick  /: search
              C-r/w/s/p: filter  Tab: worktree view  q: quit"};

        assert_eq!(output, expected);
    }

    // =========================================================================
    // Rendered tree output tests
    // =========================================================================

    #[test]
    fn test_render_parent_child_tree() {
        let now = Utc::now();

        let mut parent = create_test_session("parent");
        parent.updated_at = now;
        parent.tmux_info = Some(TmuxInfo {
            session_name: "app".to_string(),
            window_name: "main".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        parent.status = SessionStatus::Running;
        parent.current_tool = Some("Bash(cargo build)".to_string());

        let mut child = create_test_session("child");
        child.updated_at = now - Duration::minutes(2);
        child.ancestor_session_ids = vec!["parent".to_string()];
        child.tmux_info = Some(TmuxInfo {
            session_name: "app".to_string(),
            window_name: "test".to_string(),
            window_index: 1,
            pane_id: "%1".to_string(),
        });
        child.status = SessionStatus::Running;
        child.current_tool = Some("Bash(cargo test)".to_string());

        let sessions = vec![parent, child];
        let output = render_to_string(&sessions, Some(0), now, 80, 13);

        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 2  ◐ 0  ⏸ 0  ○ 0               │
            └──────────────────────────────────────────────────────────────────────────────┘
            >● ▎ project  just now
               ▎ Bash(cargo build)
             │
             └── ● ▎ project  2m ago
                   ▎ Bash(cargo test)



              j/k: move  f: focus  r: resume  p: preview  d: delete  1-9: quick  /: search
              C-r/w/s/p: filter  Tab: worktree view  q: quit"};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_multiple_children_tree() {
        let now = Utc::now();

        let mut parent = create_test_session("parent");
        parent.updated_at = now;
        parent.tmux_info = Some(TmuxInfo {
            session_name: "app".to_string(),
            window_name: "main".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });

        let mut child1 = create_test_session("child1");
        child1.updated_at = now - Duration::minutes(1);
        child1.ancestor_session_ids = vec!["parent".to_string()];
        child1.tmux_info = Some(TmuxInfo {
            session_name: "app".to_string(),
            window_name: "test".to_string(),
            window_index: 1,
            pane_id: "%1".to_string(),
        });

        let mut child2 = create_test_session("child2");
        child2.updated_at = now - Duration::minutes(3);
        child2.ancestor_session_ids = vec!["parent".to_string()];
        child2.tmux_info = Some(TmuxInfo {
            session_name: "app".to_string(),
            window_name: "review".to_string(),
            window_index: 2,
            pane_id: "%2".to_string(),
        });
        child2.status = SessionStatus::WaitingInput;

        let sessions = vec![parent, child1, child2];
        let output = render_to_string(&sessions, Some(0), now, 80, 16);

        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 2  ◐ 1  ⏸ 0  ○ 0               │
            └──────────────────────────────────────────────────────────────────────────────┘
            >● ▎ project  just now
               ▎
             │
             ├── ● ▎ project  1m ago
             │     ▎
             │
             └── ◐ ▎ project  3m ago
                   ▎



              j/k: move  f: focus  r: resume  p: preview  d: delete  1-9: quick  /: search
              C-r/w/s/p: filter  Tab: worktree view  q: quit"};

        assert_eq!(output, expected);
    }

    // =========================================================================
    // highlight_matches tests
    // =========================================================================

    /// Each expected element is (content, is_highlighted).
    #[rstest]
    #[case::empty_query("webapp:dev", "", &[("webapp:dev", false)])]
    #[case::no_match("webapp:dev", "xyz", &[("webapp:dev", false)])]
    #[case::single_word("webapp:dev", "web", &[("web", true), ("app:dev", false)])]
    #[case::case_insensitive("WebApp", "web", &[("Web", true), ("App", false)])]
    #[case::multiple_words("webapp:dev", "web dev", &[("web", true), ("app:", false), ("dev", true)])]
    #[case::overlapping_ranges("abcd", "ab bc", &[("abc", true), ("d", false)])]
    #[case::multiple_occurrences("abcabc", "ab", &[("ab", true), ("c", false), ("ab", true), ("c", false)])]
    #[case::empty_text("", "web", &[("", false)])]
    #[case::unicode_byte_length_increase("İstanbul City", "city", &[("İstanbul ", false), ("City", true)])]
    #[case::unicode_byte_length_decrease("\u{212A}elvin", "kelvin", &[("\u{212A}elvin", true)])]
    #[case::multibyte_overlapping_match("ああいああ", "ああ", &[("ああ", true), ("い", false), ("ああ", true)])]
    fn test_highlight_matches(
        #[case] text: &str,
        #[case] query: &str,
        #[case] expected: &[(&str, bool)],
    ) {
        let base = Style::default().add_modifier(Modifier::BOLD);
        let highlight = base.fg(Color::Yellow).add_modifier(Modifier::BOLD);

        let spans = highlight_matches(text, query, base);

        assert_eq!(spans.len(), expected.len(), "span count mismatch");
        for (span, &(content, is_highlighted)) in spans.iter().zip(expected) {
            assert_eq!(span.content, content);
            assert_eq!(span.style, if is_highlighted { highlight } else { base });
        }
    }

    // =========================================================================
    // Repo label tests
    // =========================================================================

    #[test]
    fn test_repo_label_color_is_deterministic() {
        let color1 = repo_label_color("armyknife");
        let color2 = repo_label_color("armyknife");
        assert_eq!(color1, color2);
    }

    #[test]
    fn test_repo_label_color_differs_for_different_names() {
        let color1 = repo_label_color("armyknife");
        let color2 = repo_label_color("specs");
        assert_ne!(color1, color2);
    }

    #[test]
    fn test_repo_label_color_returns_rgb() {
        let names = ["armyknife", "webapp", "api", "docs", "infra", "tools"];
        for name in names {
            let color = repo_label_color(name);
            assert!(
                matches!(color, Color::Rgb(_, _, _)),
                "{name} got {color:?}, expected Rgb"
            );
        }
    }

    // =========================================================================
    // Worktree view rendering
    // =========================================================================

    use crate::commands::cc::tui::worktree_view::WorktreeRow;

    fn wt_row(repo: &str, branch: &str, name: &str, path: &str) -> WorktreeRow {
        WorktreeRow {
            repo: repo.to_string(),
            branch: branch.to_string(),
            name: name.to_string(),
            path: PathBuf::from(path),
            session_count: 0,
            has_active: false,
            sessions: Vec::new(),
        }
    }

    #[test]
    fn test_render_worktree_view_grouped_rows_snapshot() {
        // Locks in row layout: header title, repo group, status glyph,
        // and crucially the column alignment between line 1 (header +
        // bar) and line 2 (bar + detail). If the bar drifts between
        // lines, the snapshot diff makes it obvious.
        let now = Utc::now();
        let output = render_to_string_with(&[], None, now, 80, 12, |app| {
            app.view = View::Worktree;
            app.set_worktrees(vec![
                wt_row(
                    "armyknife",
                    "feat/a",
                    "feat-a",
                    "/tmp/armyknife/.worktrees/feat-a",
                ),
                wt_row("specs", "main", "main", "/tmp/specs/.worktrees/main"),
            ]);
        });

        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Worktrees                                  ● 0  ◐ 0  ⏸ 0  ○ 0               │
            └──────────────────────────────────────────────────────────────────────────────┘
             ▼ armyknife
            >  ◌ ▎ armyknife feat/a
                 ▎ 0 sessions · /tmp/armyknife/.worktrees/feat-a
             ▼ specs
               ◌ ▎ specs main
                 ▎ 0 sessions · /tmp/specs/.worktrees/main

              j/k: move  Enter/f: focus  d: delete  1-9: quick  Tab: switch view  q: quit
            "};

        assert_eq!(output, expected);
    }

    #[test]
    fn test_render_clean_view_emits_both_section_headers() {
        // Both section headers should render even when one section is
        // empty (e.g. all rows default to Kept while PR fetch loads).
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
        assert!(
            output.contains("To delete") && output.contains("Kept"),
            "expected both section headers, got:\n{output}",
        );
    }

    #[test]
    fn test_render_worktree_view_loading_snapshot() {
        let now = Utc::now();
        let output = render_to_string_with(&[], None, now, 80, 9, |app| {
            app.view = View::Worktree;
            // No set_worktrees call → state stays Loading.
        });

        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Worktrees                                  ● 0  ◐ 0  ⏸ 0  ○ 0               │
            └──────────────────────────────────────────────────────────────────────────────┘
              Loading worktrees...



              j/k: move  Enter/f: focus  d: delete  1-9: quick  Tab: switch view  q: quit
            "};

        assert_eq!(output, expected);
    }
}
