mod app;
mod event;
mod ui;

use anyhow::Result;
use crossterm::event::KeyCode;
use ratatui::DefaultTerminal;

use self::app::App;
use self::event::{AppEvent, EventHandler};
use crate::commands::cc::types::TmuxInfo;
use crate::infra::tmux;

/// Runs the TUI application.
pub fn run() -> Result<()> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal);
    ratatui::restore();
    result
}

/// Main application loop.
fn run_app(terminal: &mut DefaultTerminal) -> Result<()> {
    let mut app = App::new()?;
    let event_handler = EventHandler::new()?;

    loop {
        // Draw UI
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        // Handle events
        match event_handler.next()? {
            AppEvent::Key(key) => {
                handle_key_event(&mut app, key);
            }
            AppEvent::SessionsChanged => {
                app.reload_sessions()?;
            }
            AppEvent::Tick => {
                // Periodic refresh for relative time updates
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

/// Focuses the tmux pane specified by TmuxInfo.
fn focus_tmux_pane(info: &TmuxInfo) -> Result<()> {
    tmux::switch_to_session(&info.session_name)?;
    let window_target = format!("{}:{}", info.session_name, info.window_index);
    tmux::select_window(&window_target)?;
    tmux::select_pane(&info.pane_id)?;
    Ok(())
}

/// Handles key events.
fn handle_key_event(app: &mut App, key: KeyCode) {
    // Clear error message on any key press
    app.clear_error();

    match key {
        // Quit
        KeyCode::Char('q') | KeyCode::Esc => {
            app.quit();
        }

        // Navigation
        KeyCode::Char('j') | KeyCode::Down => {
            app.select_next();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.select_previous();
        }

        // Focus on selected session's tmux pane
        KeyCode::Enter | KeyCode::Char('f') => {
            if let Some(session) = app.selected_session()
                && let Some(ref tmux_info) = session.tmux_info
                && let Err(e) = focus_tmux_pane(tmux_info)
            {
                app.set_error(format!("Failed to focus tmux pane: {e}"));
            }
        }

        // Quick select (1-9)
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            let num = c.to_digit(10).unwrap_or(0) as usize;
            app.select_by_number(num);
        }

        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::{Session, SessionStatus};
    use chrono::Utc;
    use ratatui::widgets::ListState;
    use std::path::PathBuf;

    fn create_test_app_with_sessions(count: usize) -> App {
        let sessions: Vec<Session> = (0..count)
            .map(|i| Session {
                session_id: format!("session-{}", i),
                cwd: PathBuf::from(format!("/project/{}", i)),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Running,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                last_message: None,
                current_tool: None,
            })
            .collect();

        let mut list_state = ListState::default();
        if !sessions.is_empty() {
            list_state.select(Some(0));
        }

        App {
            sessions,
            list_state,
            should_quit: false,
            error_message: None,
        }
    }

    #[test]
    fn test_handle_key_quit() {
        let mut app = create_test_app_with_sessions(1);

        handle_key_event(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }

    #[test]
    fn test_handle_key_quit_esc() {
        let mut app = create_test_app_with_sessions(1);

        handle_key_event(&mut app, KeyCode::Esc);
        assert!(app.should_quit);
    }

    #[test]
    fn test_handle_key_navigation_j() {
        let mut app = create_test_app_with_sessions(3);
        assert_eq!(app.list_state.selected(), Some(0));

        handle_key_event(&mut app, KeyCode::Char('j'));
        assert_eq!(app.list_state.selected(), Some(1));
    }

    #[test]
    fn test_handle_key_navigation_k() {
        let mut app = create_test_app_with_sessions(3);
        app.list_state.select(Some(2));

        handle_key_event(&mut app, KeyCode::Char('k'));
        assert_eq!(app.list_state.selected(), Some(1));
    }

    #[test]
    fn test_handle_key_navigation_arrows() {
        let mut app = create_test_app_with_sessions(3);
        assert_eq!(app.list_state.selected(), Some(0));

        handle_key_event(&mut app, KeyCode::Down);
        assert_eq!(app.list_state.selected(), Some(1));

        handle_key_event(&mut app, KeyCode::Up);
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn test_handle_key_quick_select() {
        let mut app = create_test_app_with_sessions(5);
        assert_eq!(app.list_state.selected(), Some(0));

        handle_key_event(&mut app, KeyCode::Char('3'));
        assert_eq!(app.list_state.selected(), Some(2));

        handle_key_event(&mut app, KeyCode::Char('1'));
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn test_handle_key_quick_select_zero_ignored() {
        let mut app = create_test_app_with_sessions(5);
        app.list_state.select(Some(2));

        handle_key_event(&mut app, KeyCode::Char('0'));
        // Should remain unchanged
        assert_eq!(app.list_state.selected(), Some(2));
    }
}
