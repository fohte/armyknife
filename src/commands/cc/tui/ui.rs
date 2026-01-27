use crate::commands::cc::claude_sessions;
use crate::commands::cc::types::{Session, SessionStatus};
use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::App;

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

    // Add error area if there's an error message
    let has_error = app.error_message.is_some();
    let layouts: Vec<Constraint> = if has_error {
        vec![
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]
    } else {
        vec![
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ]
    };

    let areas = Layout::vertical(layouts).split(area);

    render_header(frame, areas[0], &app.sessions);
    render_session_list(frame, areas[1], app, now);
    render_help(frame, areas[2]);

    if has_error {
        render_error(frame, areas[3], app.error_message.as_deref().unwrap_or(""));
    }
}

/// Renders the header with status counts.
fn render_header(frame: &mut Frame, area: Rect, sessions: &[Session]) {
    let (running, waiting, stopped) = count_statuses(sessions);

    let status_line = Line::from(vec![
        Span::styled(
            "  Claude Code Sessions",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("                       "),
        Span::styled(
            format!("{} {}", SessionStatus::Running.display_symbol(), running),
            Style::default().fg(Color::Green),
        ),
        Span::raw("  "),
        Span::styled(
            format!(
                "{} {}",
                SessionStatus::WaitingInput.display_symbol(),
                waiting
            ),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{} {}", SessionStatus::Stopped.display_symbol(), stopped),
            Style::default().fg(Color::DarkGray),
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
    render_session_list_internal(frame, area, &app.sessions, &mut app.list_state, now);
}

/// Renders the help bar at the bottom.
fn render_help(frame: &mut Frame, area: Rect) {
    let help_text = Line::from(vec![
        Span::styled("  j/k", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(": move  "),
        Span::styled("Enter/f", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(": focus  "),
        Span::styled("1-9", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(": quick select  "),
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(": quit"),
    ]);

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
    now: DateTime<Utc>,
    term_width: usize,
) -> ListItem<'static> {
    let status_symbol = session.status.display_symbol();
    let status_color = status_color(session.status);
    let session_info = get_session_info(session);
    let title = get_title_display_name(session);
    let time_ago = format_relative_time(session.updated_at, now);

    let session_info_width = calculate_session_info_width(term_width);
    let title_width = calculate_title_width(term_width);

    // First line: [number] status session:window time
    let line1 = Line::from(vec![
        Span::raw(format!("  [{}] ", index + 1)),
        Span::styled(status_symbol, Style::default().fg(status_color)),
        Span::raw(" "),
        Span::styled(
            truncate(&session_info, session_info_width),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(time_ago, Style::default().fg(Color::DarkGray)),
    ]);

    // Second line: title (from Claude Code session)
    let line2 = Line::from(vec![
        Span::raw("      "),
        Span::styled(
            truncate(&title, title_width),
            Style::default().fg(Color::Gray),
        ),
    ]);

    // Empty line for spacing
    let line3 = Line::from("");

    ListItem::new(vec![line1, line2, line3])
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

/// Gets the title display name for a session.
/// Fetches from Claude Code's sessions-index.json, falls back to tmux session:window or cwd.
fn get_title_display_name(session: &Session) -> String {
    if let Some(title) = claude_sessions::get_session_title(&session.cwd, &session.session_id) {
        return title;
    }

    if let Some(ref tmux_info) = session.tmux_info {
        return format!("{}:{}", tmux_info.session_name, tmux_info.window_name);
    }

    // Extract last component of cwd path
    session
        .cwd
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from)
        .unwrap_or_else(|| session.cwd.display().to_string())
}

/// Gets the session info (tmux session:window or cwd path).
fn get_session_info(session: &Session) -> String {
    if let Some(ref tmux_info) = session.tmux_info {
        format!("{}:{}", tmux_info.session_name, tmux_info.window_name)
    } else {
        session.cwd.display().to_string()
    }
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

    terminal
        .draw(|frame| {
            let area = frame.area();
            let areas = Layout::vertical([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);

            render_header(frame, areas[0], sessions);
            render_session_list_internal(frame, areas[1], sessions, &mut list_state, now);
            render_help(frame, areas[2]);
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

/// Internal render function for session list used by both main render and test render.
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
        .map(|(i, session)| create_session_item(i, session, now, term_width))
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
        // When sessions-index.json doesn't exist, falls back to tmux info
        let mut session = create_test_session("test");
        session.tmux_info = Some(TmuxInfo {
            session_name: "dev".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        assert_eq!(get_title_display_name(&session), "dev:editor");
    }

    #[test]
    fn test_get_title_display_name_fallback_to_cwd() {
        // When sessions-index.json doesn't exist and no tmux, falls back to cwd
        let session = create_test_session("test");
        assert_eq!(get_title_display_name(&session), "project");
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
        let output = render_to_string(&sessions, Some(0), now, 60, 15);

        // Note: ratatui's highlight_symbol ">" adds an extra space before it
        // and ListItem with 3 lines creates extra vertical spacing
        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 1  ◐ 1  ○ 1│
            └──────────────────────────────────────────────────────────┘
            >  [1] ● webapp:dev  just now
                   webapp:dev

               [2] ◐ api:test  5m ago
                   api:test

               [3] ○ /home/user/docs  1h ago
                   docs



              j/k: move  Enter/f: focus  1-9: quick select  q: quit"
        };

        assert_eq!(output, expected.trim_end());
    }

    #[test]
    fn test_render_full_screen_empty_sessions() {
        let now = Utc::now();
        let sessions: Vec<Session> = vec![];
        let output = render_to_string(&sessions, None, now, 60, 8);

        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 0  ◐ 0  ○ 0│
            └──────────────────────────────────────────────────────────┘
              No active Claude Code sessions.



              j/k: move  Enter/f: focus  1-9: quick select  q: quit"
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
        let output = render_to_string(&sessions, Some(1), now, 60, 12);

        let expected = indoc! {"
            ┌──────────────────────────────────────────────────────────┐
            │  Claude Code Sessions                       ● 1  ◐ 1  ○ 0│
            └──────────────────────────────────────────────────────────┘
               [1] ● webapp:dev  just now
                   webapp:dev

            >  [2] ◐ api:test  5m ago
                   api:test



              j/k: move  Enter/f: focus  1-9: quick select  q: quit"
        };

        assert_eq!(output, expected.trim_end());
    }
}
