use crate::commands::cc::types::{Session, SessionStatus};
use chrono::{DateTime, Utc};
#[cfg(test)]
use ratatui::widgets::ListState;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{App, AppMode};

/// Fixed width for prefix: "  [N] ● " = 8 chars
const LINE1_PREFIX_WIDTH: usize = 8;
/// Fixed width for time suffix: "  XXm ago" = ~10 chars
const LINE1_SUFFIX_WIDTH: usize = 12;
/// Fixed width for title prefix: "      " = 6 chars
const LINE2_PREFIX_WIDTH: usize = 6;
/// Minimum width for session info
const MIN_SESSION_INFO_WIDTH: usize = 20;
/// Minimum width for title
const MIN_TITLE_WIDTH: usize = 20;

/// Renders the entire UI.
pub fn render(frame: &mut Frame, app: &mut App) {
    let now = Utc::now();
    let area = frame.area();

    // Determine layout based on mode and error state
    let has_error = app.error_message.is_some();
    let is_search_mode = app.mode == AppMode::Search;
    let show_search_bar = is_search_mode || app.has_filter();

    let layouts: Vec<Constraint> = match (show_search_bar, has_error) {
        (true, true) => vec![
            Constraint::Length(3), // Header
            Constraint::Length(1), // Search bar (at top)
            Constraint::Min(1),    // Session list
            Constraint::Length(1), // Help bar
            Constraint::Length(1), // Error
        ],
        (true, false) => vec![
            Constraint::Length(3), // Header
            Constraint::Length(1), // Search bar (at top)
            Constraint::Min(1),    // Session list
            Constraint::Length(1), // Help bar
        ],
        (false, true) => vec![
            Constraint::Length(3), // Header
            Constraint::Min(1),    // Session list
            Constraint::Length(1), // Help bar
            Constraint::Length(1), // Error
        ],
        (false, false) => vec![
            Constraint::Length(3), // Header
            Constraint::Min(1),    // Session list
            Constraint::Length(1), // Help bar
        ],
    };

    let areas = Layout::vertical(layouts).split(area);

    render_header(frame, areas[0], app);

    match (show_search_bar, has_error) {
        (true, true) => {
            render_search_input(frame, areas[1], app);
            render_session_list(frame, areas[2], app, now);
            render_help(frame, areas[3], app);
            render_error(frame, areas[4], app.error_message.as_deref().unwrap_or(""));
        }
        (true, false) => {
            render_search_input(frame, areas[1], app);
            render_session_list(frame, areas[2], app, now);
            render_help(frame, areas[3], app);
        }
        (false, true) => {
            render_session_list(frame, areas[1], app, now);
            render_help(frame, areas[2], app);
            render_error(frame, areas[3], app.error_message.as_deref().unwrap_or(""));
        }
        (false, false) => {
            render_session_list(frame, areas[1], app, now);
            render_help(frame, areas[2], app);
        }
    }
}

/// Renders the header with status counts.
/// When a status filter is active, the matching status is visually highlighted.
fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let (running, waiting, stopped) = count_statuses(&app.sessions);
    let status_filter = app.status_filter;

    // Highlight active filter with underline + brighter color
    let running_style = if status_filter == Some(SessionStatus::Running) {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        Style::default().fg(Color::Green)
    };
    let waiting_style = if status_filter == Some(SessionStatus::WaitingInput) {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        Style::default().fg(Color::Yellow)
    };
    let stopped_style = if status_filter == Some(SessionStatus::Stopped) {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let status_line = Line::from(vec![
        Span::styled(
            "  Claude Code Sessions",
            Style::default().add_modifier(Modifier::BOLD),
        ),
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

/// Renders the session list.
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

    // Determine the active search query for highlighting
    let query = if app.mode == AppMode::Search {
        &app.search_query
    } else {
        &app.confirmed_query
    };

    let items: Vec<ListItem> = filtered_sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let cached_title = app.get_cached_title(&session.session_id);
            create_session_item(i, session, cached_title, now, term_width, query)
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">");

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

/// Renders the help bar at the bottom.
fn render_help(frame: &mut Frame, area: Rect, app: &App) {
    let bold = Style::default().add_modifier(Modifier::BOLD);

    let help_text = match app.mode {
        AppMode::Search => Line::from(vec![
            Span::styled("  C-n/C-p", bold),
            Span::raw(": move  "),
            Span::styled("Enter", bold),
            Span::raw(": focus  "),
            Span::styled("Esc", bold),
            Span::raw(": cancel"),
        ]),
        AppMode::Normal if app.has_filter() => Line::from(vec![
            Span::styled("  j/k", bold),
            Span::raw(": move  "),
            Span::styled("Enter/f", bold),
            Span::raw(": focus  "),
            Span::styled("/", bold),
            Span::raw(": edit  "),
            Span::styled("r/w/s", bold),
            Span::raw(": filter  "),
            Span::styled("Esc", bold),
            Span::raw(": clear  "),
            Span::styled("q", bold),
            Span::raw(": quit"),
        ]),
        AppMode::Normal => Line::from(vec![
            Span::styled("  j/k", bold),
            Span::raw(": move  "),
            Span::styled("Enter/f", bold),
            Span::raw(": focus  "),
            Span::styled("1-9", bold),
            Span::raw(": quick  "),
            Span::styled("/", bold),
            Span::raw(": search  "),
            Span::styled("r/w/s", bold),
            Span::raw(": filter  "),
            Span::styled("q", bold),
            Span::raw(": quit"),
        ]),
    };

    let help = Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray));
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

    // Build status filter badge
    let status_badge = app
        .status_filter
        .map(|s| format!("[{}] ", s.display_name()));
    let status_badge_str = status_badge.as_deref().unwrap_or("");

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
    let fixed_width = prefix.len() + status_badge_str.len() + cursor_str.len() + count_width + 2; // +2 for spacing
    let query_max_width = term_width.saturating_sub(fixed_width);

    // Truncate query if needed
    let display_query = truncate(query, query_max_width);

    // Calculate padding to right-align the count
    let content_width =
        prefix.len() + status_badge_str.len() + display_query.width() + cursor_str.len();
    let padding_width = term_width.saturating_sub(content_width + count_width + 2);
    let padding = " ".repeat(padding_width);

    let mut spans = vec![Span::styled(prefix, Style::default().fg(Color::Yellow))];

    // Show status filter badge if active
    if !status_badge_str.is_empty() {
        spans.push(Span::styled(
            status_badge_str.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }

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

/// Calculates the available width for session info on line 1.
fn calculate_session_info_width(term_width: usize) -> usize {
    let fixed_width = LINE1_PREFIX_WIDTH + LINE1_SUFFIX_WIDTH;
    if term_width > fixed_width + MIN_SESSION_INFO_WIDTH {
        term_width - fixed_width
    } else {
        MIN_SESSION_INFO_WIDTH
    }
}

/// Calculates the available width for title on line 2.
fn calculate_title_width(term_width: usize) -> usize {
    if term_width > LINE2_PREFIX_WIDTH + MIN_TITLE_WIDTH {
        term_width - LINE2_PREFIX_WIDTH
    } else {
        MIN_TITLE_WIDTH
    }
}

/// Creates a list item for a session.
fn create_session_item(
    index: usize,
    session: &Session,
    cached_title: Option<&str>,
    now: DateTime<Utc>,
    term_width: usize,
    query: &str,
) -> ListItem<'static> {
    let status_symbol = session.status.display_symbol();
    let status_color = status_color(session.status);
    let session_info = get_session_info(session);
    let title = cached_title
        .map(String::from)
        .unwrap_or_else(|| get_title_display_name_fallback(session));
    let time_ago = format_relative_time(session.updated_at, now);

    let session_info_width = calculate_session_info_width(term_width);
    let title_width = calculate_title_width(term_width);

    // First line: [number] status session:window time
    let truncated_info = truncate(&session_info, session_info_width);
    let info_style = Style::default().add_modifier(Modifier::BOLD);
    let mut line1_spans = vec![
        Span::raw(format!("  [{}] ", index + 1)),
        Span::styled(status_symbol, Style::default().fg(status_color)),
        Span::raw(" "),
    ];
    line1_spans.extend(highlight_matches(&truncated_info, query, info_style));
    line1_spans.push(Span::raw("  "));
    line1_spans.push(Span::styled(time_ago, Style::default().fg(Color::DarkGray)));
    let line1 = Line::from(line1_spans);

    // Second line: title (from Claude Code session)
    let truncated_title = truncate(&title, title_width);
    let title_style = Style::default().fg(Color::Gray);
    let mut line2_spans = vec![Span::raw("      ")];
    line2_spans.extend(highlight_matches(&truncated_title, query, title_style));
    let line2 = Line::from(line2_spans);

    // Third line: current tool (if running) or last assistant message
    let line3_content = session
        .current_tool
        .as_deref()
        .or(session.last_message.as_deref())
        .unwrap_or("");
    let truncated_line3 = truncate(line3_content, title_width);
    let line3_style = Style::default().add_modifier(Modifier::DIM);
    let mut line3_spans = vec![Span::raw("      ")];
    line3_spans.extend(highlight_matches(&truncated_line3, query, line3_style));
    let line3 = Line::from(line3_spans);

    // Empty line for spacing
    let line4 = Line::from("");

    ListItem::new(vec![line1, line2, line3, line4])
}

/// Returns the color for a session status.
fn status_color(status: SessionStatus) -> Color {
    match status {
        SessionStatus::Running => Color::Green,
        SessionStatus::WaitingInput => Color::Yellow,
        SessionStatus::Stopped => Color::DarkGray,
    }
}

/// Counts sessions by status.
fn count_statuses(sessions: &[Session]) -> (usize, usize, usize) {
    let mut running = 0;
    let mut waiting = 0;
    let mut stopped = 0;

    for session in sessions {
        match session.status {
            SessionStatus::Running => running += 1,
            SessionStatus::WaitingInput => waiting += 1,
            SessionStatus::Stopped => stopped += 1,
        }
    }

    (running, waiting, stopped)
}

/// Gets the title display name for a session without external file I/O.
/// Used as fallback when title is not in cache.
/// All outputs are sanitized to strip ANSI escape sequences.
fn get_title_display_name_fallback(session: &Session) -> String {
    use crate::commands::cc::claude_sessions;

    if let Some(ref tmux_info) = session.tmux_info {
        return claude_sessions::normalize_title(&format!(
            "{}:{}",
            tmux_info.session_name, tmux_info.window_name
        ));
    }

    // Extract last component of cwd path
    let raw_title = session
        .cwd
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from)
        .unwrap_or_else(|| session.cwd.display().to_string());
    claude_sessions::normalize_title(&raw_title)
}

/// Gets the session info (tmux session:window or cwd path).
/// All outputs are sanitized to strip ANSI escape sequences.
fn get_session_info(session: &Session) -> String {
    use crate::commands::cc::claude_sessions;

    let raw = if let Some(ref tmux_info) = session.tmux_info {
        format!("{}:{}", tmux_info.session_name, tmux_info.window_name)
    } else {
        session.cwd.display().to_string()
    };
    claude_sessions::normalize_title(&raw)
}

/// Formats a datetime as a relative time string.
fn format_relative_time(dt: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let duration = now.signed_duration_since(dt);

    let seconds = duration.num_seconds();
    if seconds < 0 {
        return "just now".to_string();
    }

    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;

    if seconds < 60 {
        "just now".to_string()
    } else if minutes < 60 {
        format!("{}m ago", minutes)
    } else if hours < 24 {
        format!("{}h ago", hours)
    } else {
        format!("{}d ago", days)
    }
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
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();

    // Create a minimal App state for rendering
    let mut list_state = ListState::default();
    list_state.select(selected_index);

    let mut app = App::with_sessions(sessions.to_vec());
    app.list_state = list_state;

    terminal
        .draw(|frame| {
            let area = frame.area();
            let areas = Layout::vertical([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);

            render_header(frame, areas[0], &app);
            render_session_list_internal(frame, areas[1], sessions, &mut app.list_state, now);
            render_help(frame, areas[2], &app);
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

/// Internal render function for session list used by test render.
#[cfg(test)]
fn render_session_list_internal(
    frame: &mut Frame,
    area: Rect,
    sessions: &[Session],
    list_state: &mut ListState,
    now: DateTime<Utc>,
) {
    if sessions.is_empty() {
        let empty_message = Paragraph::new("  No active Claude Code sessions.")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty_message, area);
        return;
    }

    let term_width = area.width as usize;
    let items: Vec<ListItem> = sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            // Use fallback for tests (no cache available), no highlight
            let title = get_title_display_name_fallback(session);
            create_session_item(i, session, Some(&title), now, term_width, "")
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">");

    frame.render_stateful_widget(list, area, list_state);
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

        let (running, waiting, stopped) = count_statuses(&sessions);
        assert_eq!(running, 2);
        assert_eq!(waiting, 1);
        assert_eq!(stopped, 1);
    }

    #[test]
    fn test_get_title_display_name_fallback_to_tmux() {
        // When cache is not available, falls back to tmux info
        let mut session = create_test_session("test");
        session.tmux_info = Some(TmuxInfo {
            session_name: "dev".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        assert_eq!(get_title_display_name_fallback(&session), "dev:editor");
    }

    #[test]
    fn test_get_title_display_name_fallback_to_cwd() {
        // When cache is not available and no tmux, falls back to cwd
        let session = create_test_session("test");
        assert_eq!(get_title_display_name_fallback(&session), "project");
    }

    #[test]
    fn test_get_session_info_with_tmux() {
        let mut session = create_test_session("test");
        session.tmux_info = Some(TmuxInfo {
            session_name: "dev".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        assert_eq!(get_session_info(&session), "dev:editor");
    }

    #[test]
    fn test_get_session_info_without_tmux() {
        let session = create_test_session("test");
        assert_eq!(get_session_info(&session), "/home/user/project");
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
    }

    // =========================================================================
    // Integration tests for session display rendering
    // =========================================================================

    #[rstest]
    #[case::narrow(40, 20, 34)]
    #[case::medium(80, 60, 74)]
    #[case::wide(120, 100, 114)]
    fn test_width_calculations(
        #[case] term_width: usize,
        #[case] expected_session_info_width: usize,
        #[case] expected_title_width: usize,
    ) {
        assert_eq!(
            calculate_session_info_width(term_width),
            expected_session_info_width
        );
        assert_eq!(calculate_title_width(term_width), expected_title_width);
    }

    // =========================================================================
    // Full-screen integration tests using TestBackend
    // =========================================================================

    #[test]
    fn test_render_full_screen_with_sessions() {
        let now = Utc::now();

        // Session 1: Running with tmux
        let mut session1 = create_test_session("s1");
        session1.updated_at = now;
        session1.tmux_info = Some(TmuxInfo {
            session_name: "webapp".to_string(),
            window_name: "dev".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        session1.status = SessionStatus::Running;

        // Session 2: WaitingInput with tmux
        let mut session2 = create_test_session("s2");
        session2.updated_at = now - Duration::minutes(5);
        session2.tmux_info = Some(TmuxInfo {
            session_name: "api".to_string(),
            window_name: "test".to_string(),
            window_index: 1,
            pane_id: "%1".to_string(),
        });
        session2.status = SessionStatus::WaitingInput;

        // Session 3: Stopped without tmux
        let mut session3 = create_test_session("s3");
        session3.cwd = PathBuf::from("/home/user/docs");
        session3.updated_at = now - Duration::hours(1);
        session3.status = SessionStatus::Stopped;

        let sessions = vec![session1, session2, session3];
        // Height increased to accommodate 4 lines per session (info + title + last_message + spacing)
        let output = render_to_string(&sessions, Some(0), now, 80, 20);

        // Note: ratatui's highlight_symbol ">" adds an extra space before it
        // ListItem with 4 lines: info + title + last_message (empty) + spacing
        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 1  ◐ 1  ○ 1                    │
            └──────────────────────────────────────────────────────────────────────────────┘
            >  [1] ● webapp:dev  just now
                   webapp:dev


               [2] ◐ api:test  5m ago
                   api:test


               [3] ○ /home/user/docs  1h ago
                   docs






              j/k: move  Enter/f: focus  1-9: quick  /: search  r/w/s: filter  q: quit"
        };

        assert_eq!(output, expected.trim_end());
    }

    #[test]
    fn test_render_full_screen_empty_sessions() {
        let now = Utc::now();
        let sessions: Vec<Session> = vec![];
        let output = render_to_string(&sessions, None, now, 80, 8);

        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 0  ◐ 0  ○ 0                    │
            └──────────────────────────────────────────────────────────────────────────────┘
              No active Claude Code sessions.



              j/k: move  Enter/f: focus  1-9: quick  /: search  r/w/s: filter  q: quit"
        };

        assert_eq!(output, expected.trim_end());
    }

    #[test]
    fn test_render_full_screen_narrow_terminal() {
        let now = Utc::now();

        let mut session = create_test_session("s1");
        session.updated_at = now;
        session.tmux_info = Some(TmuxInfo {
            session_name: "very-long-session-name".to_string(),
            window_name: "window".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        session.status = SessionStatus::Running;

        let sessions = vec![session];
        let output = render_to_string(&sessions, Some(0), now, 40, 8);

        // Note: header status counts get truncated on narrow terminal
        let expected = indoc! {"
            ┌──────────────────────────────────────┐
            │  Claude Code Sessions                │
            └──────────────────────────────────────┘
            >  [1] ● very-long-session...  just now
                   very-long-session-name:window


              j/k: move  Enter/f: focus  1-9: quick"
        };

        assert_eq!(output, expected.trim_end());
    }

    #[test]
    fn test_render_full_screen_selection_middle() {
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
        // Select the second session (index 1)
        // Height increased to accommodate 4 lines per session
        let output = render_to_string(&sessions, Some(1), now, 80, 15);

        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 1  ◐ 1  ○ 0                    │
            └──────────────────────────────────────────────────────────────────────────────┘
               [1] ● webapp:dev  just now
                   webapp:dev


            >  [2] ◐ api:test  5m ago
                   api:test





              j/k: move  Enter/f: focus  1-9: quick  /: search  r/w/s: filter  q: quit"
        };

        assert_eq!(output, expected.trim_end());
    }

    // =========================================================================
    // Integration tests for last_message display (3-line format)
    // =========================================================================

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
        let output = render_to_string(&sessions, Some(0), now, 80, 10);

        // The third line should contain the last_message
        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 1  ◐ 0  ○ 0                    │
            └──────────────────────────────────────────────────────────────────────────────┘
            >  [1] ● webapp:dev  just now
                   webapp:dev
                   I've updated the code as requested.



              j/k: move  Enter/f: focus  1-9: quick  /: search  r/w/s: filter  q: quit"
        };

        assert_eq!(output, expected.trim_end());
    }

    #[test]
    fn test_render_session_with_long_last_message_truncated() {
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
        session.last_message = Some(
            "This is a very long message that should be truncated when displayed in narrow terminal"
                .to_string(),
        );

        let sessions = vec![session];
        // Use wider terminal to show full truncation
        let output = render_to_string(&sessions, Some(0), now, 80, 11);

        // Verify the last_message line is truncated with "..."
        // title_width = 80 - 6 = 74, so the message is truncated to 71 chars + "..."
        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 1  ◐ 0  ○ 0                    │
            └──────────────────────────────────────────────────────────────────────────────┘
            >  [1] ● webapp:dev  just now
                   webapp:dev
                   This is a very long message that should be truncated when displayed in ..




              j/k: move  Enter/f: focus  1-9: quick  /: search  r/w/s: filter  q: quit"
        };

        assert_eq!(output, expected.trim_end());
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
        session.last_message = None; // No last_message

        let sessions = vec![session];
        let output = render_to_string(&sessions, Some(0), now, 80, 10);

        // When no last_message, the third line should be empty
        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 1  ◐ 0  ○ 0                    │
            └──────────────────────────────────────────────────────────────────────────────┘
            >  [1] ● webapp:dev  just now
                   webapp:dev




              j/k: move  Enter/f: focus  1-9: quick  /: search  r/w/s: filter  q: quit"
        };

        assert_eq!(output, expected.trim_end());
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
}
