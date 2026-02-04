mod app;
mod event;
mod ui;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::DefaultTerminal;

use self::app::{App, AppMode};
use self::event::{AppEvent, EventHandler, KeyEvent};
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
            AppEvent::SessionsChanged(changes) => {
                app.reload_sessions(changes.as_deref())?;
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

/// Handles key events in Search mode.
fn handle_search_key_event(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        // Cancel search
        (KeyCode::Esc, _) => {
            app.cancel_search();
        }

        // Confirm search
        (KeyCode::Enter, _) => {
            app.confirm_search();
        }

        // Navigation within filtered results (Ctrl+n/p or arrow keys only)
        (KeyCode::Char('n'), KeyModifiers::CONTROL) | (KeyCode::Down, _) => {
            app.select_next();
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) | (KeyCode::Up, _) => {
            app.select_previous();
        }

        // Clear entire search query
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.update_search_query(String::new());
        }

        // Delete last word
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            let query = app.search_query.clone();
            let trimmed = query.trim_end();
            let new_query = if let Some(pos) = trimmed.rfind(char::is_whitespace) {
                trimmed[..=pos].to_string()
            } else {
                String::new()
            };
            app.update_search_query(new_query);
        }

        // Delete character
        (KeyCode::Backspace, _) => {
            let mut query = app.search_query.clone();
            query.pop();
            app.update_search_query(query);
        }

        // Add character to search query (including j/k)
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            let mut query = app.search_query.clone();
            query.push(c);
            app.update_search_query(query);
        }

        _ => {}
    }
}

/// Handles key events in Normal mode.
fn handle_normal_key_event(app: &mut App, key: KeyEvent) {
    // Clear error message on any key press
    app.clear_error();

    match key.code {
        // Enter search mode
        KeyCode::Char('/') => {
            app.enter_search_mode();
        }

        // Clear filter or quit
        KeyCode::Esc => {
            if app.has_filter() {
                app.clear_filter();
            } else {
                app.quit();
            }
        }

        // Quit
        KeyCode::Char('q') => {
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

/// Focuses the tmux pane specified by TmuxInfo.
/// Note: `select_pane` automatically switches to the window containing the pane.
fn focus_tmux_pane(info: &TmuxInfo) -> Result<()> {
    tmux::switch_to_session(&info.session_name)?;
    tmux::select_pane(&info.pane_id)?;
    Ok(())
}

/// Handles key events based on current mode.
fn handle_key_event(app: &mut App, key: KeyEvent) {
    match app.mode {
        AppMode::Normal => handle_normal_key_event(app, key),
        AppMode::Search => handle_search_key_event(app, key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::tui::app::AppMode;
    use crate::commands::cc::types::{Session, SessionStatus};
    use chrono::Utc;
    use rstest::rstest;
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

        App::with_sessions(sessions)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn key_ctrl(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
        }
    }

    fn type_string(app: &mut App, s: &str) {
        for c in s.chars() {
            handle_key_event(app, key(KeyCode::Char(c)));
        }
    }

    #[rstest]
    #[case::q(KeyCode::Char('q'))]
    #[case::esc(KeyCode::Esc)]
    fn test_handle_key_quit(#[case] code: KeyCode) {
        let mut app = create_test_app_with_sessions(1);
        handle_key_event(&mut app, key(code));
        assert!(app.should_quit);
    }

    #[rstest]
    #[case::j(KeyCode::Char('j'), Some(0), Some(1))]
    #[case::k(KeyCode::Char('k'), Some(2), Some(1))]
    #[case::down(KeyCode::Down, Some(0), Some(1))]
    #[case::up(KeyCode::Up, Some(1), Some(0))]
    fn test_handle_key_navigation(
        #[case] code: KeyCode,
        #[case] initial: Option<usize>,
        #[case] expected: Option<usize>,
    ) {
        let mut app = create_test_app_with_sessions(3);
        app.list_state.select(initial);
        handle_key_event(&mut app, key(code));
        assert_eq!(app.list_state.selected(), expected);
    }

    #[rstest]
    #[case::select_3('3', Some(0), Some(2))]
    #[case::select_1('1', Some(2), Some(0))]
    #[case::zero_ignored('0', Some(2), Some(2))]
    fn test_handle_key_quick_select(
        #[case] c: char,
        #[case] initial: Option<usize>,
        #[case] expected: Option<usize>,
    ) {
        let mut app = create_test_app_with_sessions(5);
        app.list_state.select(initial);
        handle_key_event(&mut app, key(KeyCode::Char(c)));
        assert_eq!(app.list_state.selected(), expected);
    }

    // =========================================================================
    // Search mode tests
    // =========================================================================

    #[test]
    fn test_enter_search_mode_with_slash() {
        let mut app = create_test_app_with_sessions(3);
        assert_eq!(app.mode, AppMode::Normal);

        handle_key_event(&mut app, key(KeyCode::Char('/')));
        assert_eq!(app.mode, AppMode::Search);
    }

    #[test]
    fn test_search_mode_typing() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "test");
        assert_eq!(app.search_query, "test");
    }

    #[test]
    fn test_search_mode_jk_are_typed_not_navigation() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "jk");
        // j and k should be typed as characters, not used for navigation
        assert_eq!(app.search_query, "jk");
    }

    #[test]
    fn test_search_mode_ctrl_n_p_for_navigation() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));

        // Ctrl+n should move down
        handle_key_event(&mut app, key_ctrl('n'));
        assert_eq!(app.list_state.selected(), Some(1));

        // Ctrl+p should move up
        handle_key_event(&mut app, key_ctrl('p'));
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn test_search_mode_backspace() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "tes");
        handle_key_event(&mut app, key(KeyCode::Backspace));
        assert_eq!(app.search_query, "te");
    }

    #[test]
    fn test_search_mode_ctrl_u_clears_query() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "test");
        handle_key_event(&mut app, key_ctrl('u'));
        assert_eq!(app.search_query, "");
    }

    #[test]
    fn test_search_mode_ctrl_w_deletes_word() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "hello world");
        handle_key_event(&mut app, key_ctrl('w'));
        assert_eq!(app.search_query, "hello ");
    }

    #[test]
    fn test_search_mode_confirm() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "0");
        handle_key_event(&mut app, key(KeyCode::Enter));

        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.confirmed_query, "0");
        assert!(app.has_filter());
    }

    #[test]
    fn test_search_mode_cancel() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "te");
        handle_key_event(&mut app, key(KeyCode::Esc));

        assert_eq!(app.mode, AppMode::Normal);
        assert!(!app.has_filter());
    }

    #[test]
    fn test_esc_clears_filter_in_normal_mode() {
        let mut app = create_test_app_with_sessions(3);

        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "0");
        handle_key_event(&mut app, key(KeyCode::Enter));
        assert!(app.has_filter());

        handle_key_event(&mut app, key(KeyCode::Esc));
        assert!(!app.has_filter());
        assert!(!app.should_quit);
    }

    #[test]
    fn test_esc_quits_when_no_filter() {
        let mut app = create_test_app_with_sessions(3);
        assert!(!app.has_filter());

        handle_key_event(&mut app, key(KeyCode::Esc));
        assert!(app.should_quit);
    }
}
