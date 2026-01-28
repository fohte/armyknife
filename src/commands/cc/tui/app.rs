use crate::commands::cc::claude_sessions;
use crate::commands::cc::store;
use crate::commands::cc::types::Session;
use anyhow::Result;
use ratatui::widgets::ListState;

/// Application mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppMode {
    #[default]
    Normal,
    Search,
}

/// Application state for the TUI.
pub struct App {
    /// All sessions loaded from disk.
    pub sessions: Vec<Session>,
    /// State for the list widget (tracks selected index).
    pub list_state: ListState,
    /// Whether the application should quit.
    pub should_quit: bool,
    /// Error message to display (cleared on next action).
    pub error_message: Option<String>,
    /// Current application mode.
    pub mode: AppMode,
    /// Current search query.
    pub search_query: String,
    /// Confirmed search query (applied filter).
    pub confirmed_query: String,
    /// Indices of sessions that match the current filter.
    pub filtered_indices: Vec<usize>,
    /// Selection index before entering search mode (for restoration on cancel).
    pub pre_search_selection: Option<usize>,
}

impl App {
    /// Creates a new App instance with initial session data.
    pub fn new() -> Result<Self> {
        let sessions = load_sessions()?;
        let mut list_state = ListState::default();

        // Build initial filtered indices (all sessions)
        let filtered_indices: Vec<usize> = (0..sessions.len()).collect();

        // Select first item if there are any sessions
        if !sessions.is_empty() {
            list_state.select(Some(0));
        }

        Ok(Self {
            sessions,
            list_state,
            should_quit: false,
            error_message: None,
            mode: AppMode::Normal,
            search_query: String::new(),
            confirmed_query: String::new(),
            filtered_indices,
            pre_search_selection: None,
        })
    }

    /// Reloads sessions from disk.
    /// Preserves the selection by session_id if possible.
    pub fn reload_sessions(&mut self) -> Result<()> {
        // Remember the currently selected session_id
        let selected_session_id = self.selected_session().map(|s| s.session_id.clone());

        self.sessions = load_sessions()?;

        // Re-apply filter with current query
        self.apply_filter();

        // Try to restore selection by session_id within filtered results
        if let Some(ref id) = selected_session_id
            && let Some(filtered_pos) = self
                .filtered_indices
                .iter()
                .position(|&i| self.sessions.get(i).is_some_and(|s| &s.session_id == id))
        {
            self.list_state.select(Some(filtered_pos));
            return Ok(());
        }

        // Fallback: adjust selection if needed
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else if let Some(selected) = self.list_state.selected() {
            if selected >= self.filtered_indices.len() {
                self.list_state
                    .select(Some(self.filtered_indices.len() - 1));
            }
        } else {
            self.list_state.select(Some(0));
        }

        Ok(())
    }

    /// Moves selection to the next item in filtered list.
    pub fn select_next(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.filtered_indices.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    /// Moves selection to the previous item in filtered list.
    pub fn select_previous(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.filtered_indices.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    /// Selects a session by its 1-indexed number (1-9) within filtered list.
    pub fn select_by_number(&mut self, num: usize) {
        if num > 0 && num <= self.filtered_indices.len() {
            self.list_state.select(Some(num - 1));
        }
    }

    /// Returns the currently selected session, if any.
    pub fn selected_session(&self) -> Option<&Session> {
        self.list_state
            .selected()
            .and_then(|i| self.filtered_indices.get(i))
            .and_then(|&session_idx| self.sessions.get(session_idx))
    }

    /// Returns the filtered sessions for display.
    pub fn filtered_sessions(&self) -> Vec<&Session> {
        self.filtered_indices
            .iter()
            .filter_map(|&i| self.sessions.get(i))
            .collect()
    }

    /// Returns whether a filter is currently active.
    pub fn has_filter(&self) -> bool {
        !self.confirmed_query.is_empty()
    }

    /// Signals that the application should quit.
    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Sets an error message to display.
    pub fn set_error(&mut self, message: String) {
        self.error_message = Some(message);
    }

    /// Clears the error message.
    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    /// Enters search mode.
    pub fn enter_search_mode(&mut self) {
        self.pre_search_selection = self.list_state.selected();
        self.search_query = self.confirmed_query.clone();
        self.mode = AppMode::Search;
    }

    /// Exits search mode, confirming the search.
    pub fn confirm_search(&mut self) {
        self.confirmed_query = self.search_query.clone();
        self.apply_filter();
        self.mode = AppMode::Normal;
        self.pre_search_selection = None;
    }

    /// Exits search mode, cancelling the search.
    pub fn cancel_search(&mut self) {
        self.search_query = self.confirmed_query.clone();
        self.apply_filter();
        // Restore previous selection if possible
        if let Some(prev) = self.pre_search_selection
            && prev < self.filtered_indices.len()
        {
            self.list_state.select(Some(prev));
        }
        self.mode = AppMode::Normal;
        self.pre_search_selection = None;
    }

    /// Clears the filter and shows all sessions.
    pub fn clear_filter(&mut self) {
        self.search_query.clear();
        self.confirmed_query.clear();
        self.filtered_indices = (0..self.sessions.len()).collect();
        if !self.filtered_indices.is_empty() {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
    }

    /// Updates the search query and re-applies the filter.
    pub fn update_search_query(&mut self, query: String) {
        self.search_query = query;
        self.apply_filter();
    }

    /// Applies the current search query to filter sessions.
    fn apply_filter(&mut self) {
        let query = if self.mode == AppMode::Search {
            &self.search_query
        } else {
            &self.confirmed_query
        };

        if query.is_empty() {
            self.filtered_indices = (0..self.sessions.len()).collect();
        } else {
            self.filtered_indices = self
                .sessions
                .iter()
                .enumerate()
                .filter(|(_, session)| session_matches(session, query))
                .map(|(i, _)| i)
                .collect();
        }

        // Reset selection to first item or none
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }
}

/// Checks if a session matches the search query.
/// Uses case-insensitive partial matching with AND logic for multiple words.
fn session_matches(session: &Session, query: &str) -> bool {
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return true;
    }

    // Build searchable text from session fields
    let searchable = build_searchable_text(session);
    let searchable_lower = searchable.to_lowercase();

    // All words must match (AND logic)
    words
        .iter()
        .all(|word| searchable_lower.contains(&word.to_lowercase()))
}

/// Builds a searchable text string from session fields.
fn build_searchable_text(session: &Session) -> String {
    let mut parts = Vec::new();

    // tmux session name and window name
    if let Some(ref tmux_info) = session.tmux_info {
        parts.push(tmux_info.session_name.clone());
        parts.push(tmux_info.window_name.clone());
    }

    // Working directory
    parts.push(session.cwd.display().to_string());

    // Claude Code session title
    if let Some(title) = claude_sessions::get_session_title(&session.cwd, &session.session_id) {
        parts.push(title);
    }

    // Last message
    if let Some(ref msg) = session.last_message {
        parts.push(msg.clone());
    }

    parts.join(" ")
}

/// Loads sessions from disk with cleanup.
fn load_sessions() -> Result<Vec<Session>> {
    store::cleanup_stale_sessions()?;
    store::list_sessions()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::{SessionStatus, TmuxInfo};
    use chrono::Utc;
    use std::path::PathBuf;

    fn create_test_session(id: &str) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/tmp/test"),
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

    fn create_test_app(sessions: Vec<Session>) -> App {
        let filtered_indices: Vec<usize> = (0..sessions.len()).collect();
        let mut list_state = ListState::default();
        if !sessions.is_empty() {
            list_state.select(Some(0));
        }
        App {
            sessions,
            list_state,
            should_quit: false,
            error_message: None,
            mode: AppMode::Normal,
            search_query: String::new(),
            confirmed_query: String::new(),
            filtered_indices,
            pre_search_selection: None,
        }
    }

    #[test]
    fn test_select_next_empty() {
        let mut app = create_test_app(vec![]);

        app.select_next();
        assert!(app.list_state.selected().is_none());
    }

    #[test]
    fn test_select_next_wraps() {
        let mut app = create_test_app(vec![create_test_session("1"), create_test_session("2")]);
        app.list_state.select(Some(1));

        app.select_next();
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn test_select_previous_wraps() {
        let mut app = create_test_app(vec![create_test_session("1"), create_test_session("2")]);
        app.list_state.select(Some(0));

        app.select_previous();
        assert_eq!(app.list_state.selected(), Some(1));
    }

    #[test]
    fn test_select_by_number() {
        let mut app = create_test_app(vec![
            create_test_session("1"),
            create_test_session("2"),
            create_test_session("3"),
        ]);
        app.list_state.select(Some(0));

        app.select_by_number(2);
        assert_eq!(app.list_state.selected(), Some(1));

        // Out of range should not change selection
        app.select_by_number(10);
        assert_eq!(app.list_state.selected(), Some(1));

        // Zero should not change selection
        app.select_by_number(0);
        assert_eq!(app.list_state.selected(), Some(1));
    }

    #[test]
    fn test_quit() {
        let mut app = create_test_app(vec![]);

        assert!(!app.should_quit);
        app.quit();
        assert!(app.should_quit);
    }

    #[test]
    fn test_selected_session() {
        let mut app = create_test_app(vec![
            create_test_session("first"),
            create_test_session("second"),
        ]);

        app.list_state.select(Some(1));
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("second")
        );
    }

    #[test]
    fn test_error_message() {
        let mut app = create_test_app(vec![]);

        assert!(app.error_message.is_none());

        app.set_error("Test error".to_string());
        assert_eq!(app.error_message, Some("Test error".to_string()));

        app.clear_error();
        assert!(app.error_message.is_none());
    }

    // =========================================================================
    // Search functionality tests
    // =========================================================================

    #[test]
    fn test_session_matches_empty_query() {
        let session = create_test_session("test");
        assert!(session_matches(&session, ""));
        assert!(session_matches(&session, "   "));
    }

    #[test]
    fn test_session_matches_cwd() {
        let mut session = create_test_session("test");
        session.cwd = PathBuf::from("/home/user/project");

        assert!(session_matches(&session, "project"));
        assert!(session_matches(&session, "PROJECT")); // case insensitive
        assert!(session_matches(&session, "user"));
        assert!(!session_matches(&session, "nonexistent"));
    }

    #[test]
    fn test_session_matches_tmux_info() {
        let mut session = create_test_session("test");
        session.tmux_info = Some(TmuxInfo {
            session_name: "webapp".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });

        assert!(session_matches(&session, "webapp"));
        assert!(session_matches(&session, "editor"));
        assert!(session_matches(&session, "WEBAPP")); // case insensitive
    }

    #[test]
    fn test_session_matches_last_message() {
        let mut session = create_test_session("test");
        session.last_message = Some("I've updated the code".to_string());

        assert!(session_matches(&session, "updated"));
        assert!(session_matches(&session, "code"));
    }

    #[test]
    fn test_session_matches_and_logic() {
        let mut session = create_test_session("test");
        session.cwd = PathBuf::from("/home/user/webapp");
        session.last_message = Some("Working on feature".to_string());

        // Both words must match
        assert!(session_matches(&session, "webapp feature"));
        assert!(session_matches(&session, "user working"));
        assert!(!session_matches(&session, "webapp nonexistent"));
    }

    #[test]
    fn test_enter_search_mode() {
        let mut app = create_test_app(vec![create_test_session("1"), create_test_session("2")]);
        app.list_state.select(Some(1));

        app.enter_search_mode();

        assert_eq!(app.mode, AppMode::Search);
        assert_eq!(app.pre_search_selection, Some(1));
    }

    #[test]
    fn test_confirm_search() {
        let mut session1 = create_test_session("1");
        session1.cwd = PathBuf::from("/home/user/webapp");
        let mut session2 = create_test_session("2");
        session2.cwd = PathBuf::from("/home/user/api");

        let mut app = create_test_app(vec![session1, session2]);
        app.enter_search_mode();
        app.update_search_query("webapp".to_string());
        app.confirm_search();

        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.confirmed_query, "webapp");
        assert_eq!(app.filtered_indices, vec![0]); // Only first session matches
        assert!(app.has_filter());
    }

    #[test]
    fn test_cancel_search() {
        let mut session1 = create_test_session("1");
        session1.cwd = PathBuf::from("/home/user/webapp");
        let mut session2 = create_test_session("2");
        session2.cwd = PathBuf::from("/home/user/api");

        let mut app = create_test_app(vec![session1, session2]);
        app.list_state.select(Some(1)); // Select second session
        app.enter_search_mode();
        app.update_search_query("webapp".to_string());

        // At this point, filter is applied during search
        assert_eq!(app.filtered_indices, vec![0]);

        app.cancel_search();

        // Should restore to showing all sessions
        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.filtered_indices, vec![0, 1]);
        assert!(!app.has_filter());
    }

    #[test]
    fn test_clear_filter() {
        let mut session1 = create_test_session("1");
        session1.cwd = PathBuf::from("/home/user/webapp");
        let session2 = create_test_session("2");

        let mut app = create_test_app(vec![session1, session2]);
        app.enter_search_mode();
        app.update_search_query("webapp".to_string());
        app.confirm_search();

        assert!(app.has_filter());

        app.clear_filter();

        assert!(!app.has_filter());
        assert_eq!(app.filtered_indices, vec![0, 1]);
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn test_navigation_with_filter() {
        let mut session1 = create_test_session("1");
        session1.cwd = PathBuf::from("/home/user/webapp1");
        let mut session2 = create_test_session("2");
        session2.cwd = PathBuf::from("/home/user/api");
        let mut session3 = create_test_session("3");
        session3.cwd = PathBuf::from("/home/user/webapp2");

        let mut app = create_test_app(vec![session1, session2, session3]);
        app.enter_search_mode();
        app.update_search_query("webapp".to_string());
        app.confirm_search();

        // Filter shows sessions 0 and 2
        assert_eq!(app.filtered_indices, vec![0, 2]);
        assert_eq!(app.list_state.selected(), Some(0));

        // Navigate within filtered list
        app.select_next();
        assert_eq!(app.list_state.selected(), Some(1)); // Second item in filtered list

        // Verify selected session is session3 (index 2 in original list)
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("3")
        );

        // Wrap around
        app.select_next();
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn test_select_by_number_with_filter() {
        let mut session1 = create_test_session("1");
        session1.cwd = PathBuf::from("/home/user/webapp1");
        let mut session2 = create_test_session("2");
        session2.cwd = PathBuf::from("/home/user/api");
        let mut session3 = create_test_session("3");
        session3.cwd = PathBuf::from("/home/user/webapp2");

        let mut app = create_test_app(vec![session1, session2, session3]);
        app.enter_search_mode();
        app.update_search_query("webapp".to_string());
        app.confirm_search();

        // Press "2" to select second item in filtered list
        app.select_by_number(2);
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("3")
        );

        // Out of range (only 2 items in filtered list)
        app.select_by_number(3);
        // Should remain unchanged
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("3")
        );
    }
}
