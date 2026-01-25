use crate::commands::cc::types::{Session, SessionStatus};
use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use super::app::App;

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
    if app.sessions.is_empty() {
        let empty_message = Paragraph::new("  No active Claude Code sessions.")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty_message, area);
        return;
    }

    let items: Vec<ListItem> = app
        .sessions
        .iter()
        .enumerate()
        .map(|(i, session)| create_session_item(i, session, now))
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">");

    frame.render_stateful_widget(list, area, &mut app.list_state);
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

/// Creates a list item for a session.
fn create_session_item(index: usize, session: &Session, now: DateTime<Utc>) -> ListItem<'static> {
    let status_symbol = session.status.display_symbol();
    let status_color = status_color(session.status);
    let session_name = get_session_display_name(session);
    let time_ago = format_relative_time(session.updated_at, now);

    // First line: [number] status session_name time
    let line1 = Line::from(vec![
        Span::raw(format!("  [{}] ", index + 1)),
        Span::styled(status_symbol, Style::default().fg(status_color)),
        Span::raw(" "),
        Span::styled(
            truncate(&session_name, 50),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(time_ago, Style::default().fg(Color::DarkGray)),
    ]);

    // Second line: description or last activity
    let description = get_session_description(session);
    let line2 = Line::from(vec![
        Span::raw("      "),
        Span::styled(truncate(&description, 60), Style::default().fg(Color::Gray)),
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

/// Gets the display name for a session.
fn get_session_display_name(session: &Session) -> String {
    if let Some(ref tmux_info) = session.tmux_info {
        format!("{}:{}", tmux_info.session_name, tmux_info.window_name)
    } else {
        // Extract last component of cwd path
        session
            .cwd
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| session.cwd.display().to_string())
    }
}

/// Gets a description for the session.
fn get_session_description(session: &Session) -> String {
    if let Some(ref msg) = session.last_message {
        return msg.clone();
    }

    // Default description based on status
    match session.status {
        SessionStatus::Running => format!("Working in {}", session.cwd.display()),
        SessionStatus::WaitingInput => "Waiting for input...".to_string(),
        SessionStatus::Stopped => "Session stopped".to_string(),
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

/// Truncates a string to the specified length.
fn truncate(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else if max_len < 3 {
        s.chars().take(max_len).collect()
    } else {
        let truncated: String = s.chars().take(max_len - 3).collect();
        format!("{}...", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::TmuxInfo;
    use chrono::Duration;
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
    fn test_get_session_display_name_without_tmux() {
        let session = create_test_session("test");
        assert_eq!(get_session_display_name(&session), "project");
    }

    #[test]
    fn test_get_session_display_name_with_tmux() {
        let mut session = create_test_session("test");
        session.tmux_info = Some(TmuxInfo {
            session_name: "dev".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        assert_eq!(get_session_display_name(&session), "dev:editor");
    }

    #[test]
    fn test_get_session_description_with_message() {
        let mut session = create_test_session("test");
        session.last_message = Some("Working on feature X".to_string());
        assert_eq!(get_session_description(&session), "Working on feature X");
    }

    #[test]
    fn test_get_session_description_running() {
        let session = create_test_session("test");
        assert_eq!(
            get_session_description(&session),
            "Working in /home/user/project"
        );
    }

    #[test]
    fn test_get_session_description_waiting() {
        let mut session = create_test_session("test");
        session.status = SessionStatus::WaitingInput;
        assert_eq!(get_session_description(&session), "Waiting for input...");
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
    fn test_truncate(#[case] input: &str, #[case] max_len: usize, #[case] expected: &str) {
        assert_eq!(truncate(input, max_len), expected);
    }

    #[test]
    fn test_status_color() {
        assert_eq!(status_color(SessionStatus::Running), Color::Green);
        assert_eq!(status_color(SessionStatus::WaitingInput), Color::Yellow);
        assert_eq!(status_color(SessionStatus::Stopped), Color::DarkGray);
    }
}
