mod app;
mod event;
mod ui;

use std::collections::HashMap;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::DefaultTerminal;

use self::app::{App, AppMode};
use self::event::{AppEvent, EventHandler, KeyEvent, SessionChange, SessionChangeType};
use crate::commands::cc::types::SessionStatus;
use crate::infra::tmux;

/// Runs the TUI application.
pub fn run() -> Result<()> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal);
    ratatui::restore();
    result
}

/// Maximum events to drain per iteration to prevent starvation under sustained load.
const MAX_DRAIN_PER_ITERATION: usize = 100;

/// Main application loop.
///
/// Uses an event-drain strategy to prevent queue buildup:
/// 1. Block-wait for the first event
/// 2. Drain remaining queued events (non-blocking, up to MAX_DRAIN_PER_ITERATION)
/// 3. Key events are processed immediately during drain
/// 4. SessionsChanged events are merged (deduplicated by session_id)
/// 5. The merged reload + render happens once per iteration
fn run_app(terminal: &mut DefaultTerminal) -> Result<()> {
    let mut app = App::new()?;
    let event_handler = EventHandler::new()?;

    loop {
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        let first_event = event_handler.next()?;
        let mut needs_full_reload = false;
        let mut change_map: HashMap<String, SessionChangeType> = HashMap::new();

        // Process first event + drain queued events (bounded to prevent starvation)
        for event in std::iter::once(first_event)
            .chain(std::iter::from_fn(|| event_handler.try_next()))
            .take(MAX_DRAIN_PER_ITERATION)
        {
            match event {
                AppEvent::Key(k) => handle_key_event(&mut app, k),
                AppEvent::SessionsChanged(Some(changes)) => {
                    for c in changes {
                        change_map.insert(c.session_id, c.change_type);
                    }
                }
                AppEvent::SessionsChanged(None) => needs_full_reload = true,
                AppEvent::Tick => {}
            }
        }

        // Apply merged session changes in a single reload
        if let Some(merged) = merge_session_changes(change_map, needs_full_reload) {
            if !merged.is_empty() {
                app.reload_sessions(Some(&merged))?;
            }
        } else {
            app.reload_sessions(None)?;
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

/// Merges session changes by deduplicating on session_id, keeping the last change_type.
/// Returns `None` if `needs_full_reload` is true (caller should do a full reload).
/// Returns `Some(vec)` with deduplicated changes, or `Some(empty)` if no changes.
fn merge_session_changes(
    change_map: HashMap<String, SessionChangeType>,
    needs_full_reload: bool,
) -> Option<Vec<SessionChange>> {
    if needs_full_reload {
        return None;
    }
    Some(
        change_map
            .into_iter()
            .map(|(session_id, change_type)| SessionChange {
                session_id,
                change_type,
            })
            .collect(),
    )
}

/// Focuses on the selected session's tmux pane.
fn focus_selected_session(app: &mut App) {
    if let Some(session) = app.selected_session()
        && let Some(ref tmux_info) = session.tmux_info
        && let Err(e) = tmux::focus_pane(&tmux_info.pane_id)
    {
        app.set_error(format!("Failed to focus tmux pane: {e}"));
    }
}

/// Handles key events in Search mode.
fn handle_search_key_event(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        // Cancel search
        (KeyCode::Esc, _) => {
            app.cancel_search();
        }

        // Confirm search and focus on selected session
        (KeyCode::Enter, _) => {
            app.confirm_search();
            focus_selected_session(app);
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
            focus_selected_session(app);
        }

        // Status filters (toggle)
        KeyCode::Char('w') => {
            app.toggle_status_filter(SessionStatus::WaitingInput);
        }
        KeyCode::Char('s') => {
            app.toggle_status_filter(SessionStatus::Stopped);
        }
        KeyCode::Char('r') => {
            app.toggle_status_filter(SessionStatus::Running);
        }

        // Quick select (1-9)
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            let num = c.to_digit(10).unwrap_or(0) as usize;
            app.select_by_number(num);
        }

        _ => {}
    }
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
    use rstest::{fixture, rstest};
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
                label: None,
                ancestor_session_ids: Vec::new(),
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

    // =========================================================================
    // Status filter key binding tests
    // =========================================================================

    #[fixture]
    fn app_with_statuses() -> App {
        let sessions: Vec<Session> = vec![
            Session {
                session_id: "session-running".to_string(),
                cwd: PathBuf::from("/project/running"),
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
            },
            Session {
                session_id: "session-waiting".to_string(),
                cwd: PathBuf::from("/project/waiting"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::WaitingInput,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                last_message: None,
                current_tool: None,
                label: None,
                ancestor_session_ids: Vec::new(),
            },
            Session {
                session_id: "session-stopped".to_string(),
                cwd: PathBuf::from("/project/stopped"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Stopped,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                last_message: None,
                current_tool: None,
                label: None,
                ancestor_session_ids: Vec::new(),
            },
        ];
        App::with_sessions(sessions)
    }

    #[rstest]
    #[case::w_toggles_waiting('w', SessionStatus::WaitingInput)]
    #[case::s_toggles_stopped('s', SessionStatus::Stopped)]
    #[case::r_toggles_running('r', SessionStatus::Running)]
    fn test_handle_key_status_filter(
        app_with_statuses: App,
        #[case] c: char,
        #[case] expected_status: SessionStatus,
    ) {
        let mut app = app_with_statuses;
        handle_key_event(&mut app, key(KeyCode::Char(c)));

        assert_eq!(app.status_filter, Some(expected_status));
        // All filtered sessions should have the expected status
        for session in app.filtered_sessions() {
            assert_eq!(session.status, expected_status);
        }
    }

    #[rstest]
    fn test_status_filter_toggle_off(app_with_statuses: App) {
        let mut app = app_with_statuses;

        // Press 'w' to set WaitingInput filter
        handle_key_event(&mut app, key(KeyCode::Char('w')));
        assert_eq!(app.status_filter, Some(SessionStatus::WaitingInput));
        assert_eq!(app.filtered_sessions().len(), 1);

        // Press 'w' again to clear the filter
        handle_key_event(&mut app, key(KeyCode::Char('w')));
        assert!(app.status_filter.is_none());
        assert_eq!(app.filtered_sessions().len(), 3);
    }

    #[rstest]
    fn test_esc_clears_status_filter(app_with_statuses: App) {
        let mut app = app_with_statuses;

        // Set status filter
        handle_key_event(&mut app, key(KeyCode::Char('w')));
        assert!(app.has_filter());

        // Press Esc to clear filter (should not quit because filter is active)
        handle_key_event(&mut app, key(KeyCode::Esc));
        assert!(!app.has_filter());
        assert!(app.status_filter.is_none());
        assert!(!app.should_quit);
    }

    // =========================================================================
    // merge_session_changes tests
    // =========================================================================

    #[test]
    fn test_merge_session_changes_full_reload_returns_none() {
        let map = HashMap::from([("s1".to_string(), SessionChangeType::Modified)]);
        // Even with changes in the map, full reload takes precedence
        assert!(merge_session_changes(map, true).is_none());
    }

    #[test]
    fn test_merge_session_changes_empty_map_no_reload() {
        let map = HashMap::new();
        let result = merge_session_changes(map, false);
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_merge_session_changes_dedup_same_session() {
        // Simulate: session "s1" was Created then Modified → HashMap keeps last insert
        let mut map = HashMap::new();
        map.insert("s1".to_string(), SessionChangeType::Created);
        map.insert("s1".to_string(), SessionChangeType::Modified);

        let result = merge_session_changes(map, false).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].session_id, "s1");
        assert_eq!(result[0].change_type, SessionChangeType::Modified);
    }

    #[test]
    fn test_merge_session_changes_distinct_sessions() {
        let map = HashMap::from([
            ("s1".to_string(), SessionChangeType::Created),
            ("s2".to_string(), SessionChangeType::Deleted),
            ("s3".to_string(), SessionChangeType::Modified),
        ]);

        let mut result = merge_session_changes(map, false).unwrap();
        assert_eq!(result.len(), 3);

        // Sort by session_id for deterministic verification
        result.sort_by_key(|c| c.session_id.clone());

        assert_eq!(result[0].session_id, "s1");
        assert_eq!(result[0].change_type, SessionChangeType::Created);
        assert_eq!(result[1].session_id, "s2");
        assert_eq!(result[1].change_type, SessionChangeType::Deleted);
        assert_eq!(result[2].session_id, "s3");
        assert_eq!(result[2].change_type, SessionChangeType::Modified);
    }

    #[rstest]
    #[case::created_then_deleted(
        SessionChangeType::Created,
        SessionChangeType::Deleted,
        SessionChangeType::Deleted
    )]
    #[case::modified_then_deleted(
        SessionChangeType::Modified,
        SessionChangeType::Deleted,
        SessionChangeType::Deleted
    )]
    #[case::deleted_then_created(
        SessionChangeType::Deleted,
        SessionChangeType::Created,
        SessionChangeType::Created
    )]
    fn test_merge_session_changes_last_wins(
        #[case] first: SessionChangeType,
        #[case] second: SessionChangeType,
        #[case] expected: SessionChangeType,
    ) {
        let mut map = HashMap::new();
        map.insert("s1".to_string(), first);
        map.insert("s1".to_string(), second);

        let result = merge_session_changes(map, false).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].change_type, expected);
    }

    #[test]
    fn test_merge_session_changes_full_reload_overrides_changes() {
        // Even with many changes, full reload means None
        let map = HashMap::from([
            ("s1".to_string(), SessionChangeType::Modified),
            ("s2".to_string(), SessionChangeType::Created),
            ("s3".to_string(), SessionChangeType::Deleted),
        ]);
        assert!(merge_session_changes(map, true).is_none());
    }
}
