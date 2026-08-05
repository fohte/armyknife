//! Inline rename bar shown while `AppMode::Edit` is active, rendered in the
//! same slot the search bar occupies (see `chrome::render_top_bar`).

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::commands::cc::tui::app::{App, AppMode};

/// Renders the live edit buffer with a blinking cursor. The session being
/// renamed is the one highlighted in the list below -- this bar only needs
/// to show the buffer itself, which was seeded from that session's current
/// title (see `App::enter_edit_title`).
pub(super) fn render_edit_input(frame: &mut Frame, area: Rect, app: &App) {
    let is_generating = matches!(
        &app.mode,
        AppMode::Edit { session_id } if app.title_generating.as_ref().is_some_and(|(id, _)| id == session_id)
    );

    let mut spans = vec![
        Span::styled("  Rename: ", Style::default().fg(Color::Yellow)),
        Span::raw(app.edit_title_query.clone()),
        Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
    ];
    if is_generating {
        spans.push(Span::styled(
            "  Generating title...",
            Style::default().fg(Color::DarkGray),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::commands::cc::tui::app::AppMode;
    use crate::commands::cc::tui::ui::test_support::{create_test_session, render_to_string_with};

    #[test]
    fn renders_prefix_and_buffer_with_cursor() {
        let now = Utc::now();
        let sessions = vec![create_test_session("s1")];
        let output = render_to_string_with(&sessions, Some(1), now, 80, 9, |app| {
            app.mode = AppMode::Edit {
                session_id: "s1".to_string(),
            };
            app.edit_title_query = "New Title".to_string();
        });

        let bar_line = output.lines().nth(1).unwrap();
        assert_eq!(bar_line.trim_end(), "  Rename: New Title_");
    }

    #[test]
    fn renders_generating_indicator_while_title_generation_is_in_flight() {
        let now = Utc::now();
        let sessions = vec![create_test_session("s1")];
        let output = render_to_string_with(&sessions, Some(1), now, 80, 9, |app| {
            app.mode = AppMode::Edit {
                session_id: "s1".to_string(),
            };
            app.edit_title_query = "New Title".to_string();
            app.title_generating = Some(("s1".to_string(), 1));
        });

        let bar_line = output.lines().nth(1).unwrap();
        assert_eq!(
            bar_line.trim_end(),
            "  Rename: New Title_  Generating title..."
        );
    }
}
