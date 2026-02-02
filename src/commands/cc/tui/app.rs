use crate::commands::cc::claude_sessions;
use crate::commands::cc::store;
use crate::commands::cc::types::Session;
use crate::infra::tmux;
use anyhow::Result;
use chrono::{DateTime, Utc};
use ratatui::widgets::ListState;
use std::collections::HashMap;

use super::event::{SessionChange, SessionChangeType};

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
    /// Cache of searchable text for each session (keyed by session_id).
    /// Lazily built when search mode is first entered.
    /// Stores (searchable_text, updated_at) for incremental updates.
    searchable_text_cache: Option<HashMap<String, (String, DateTime<Utc>)>>,
    /// Cache of session titles for display (keyed by session_id).
    /// Built on load/reload for fast UI rendering.
    title_cache: HashMap<String, String>,
}

impl App {
    /// Creates a new App instance with initial session data.
    pub fn new() -> Result<Self> {
        let sessions = load_sessions()?;
        Ok(Self::with_sessions(sessions))
    }

    /// Creates a new App instance with the given sessions.
    /// Useful for testing without disk I/O.
    pub fn with_sessions(sessions: Vec<Session>) -> Self {
        let mut list_state = ListState::default();

        // Build initial filtered indices (all sessions)
        let filtered_indices: Vec<usize> = (0..sessions.len()).collect();

        // Build title cache for fast UI rendering
        let title_cache = build_title_cache(&sessions);

        // Select first item if there are any sessions
        if !sessions.is_empty() {
            list_state.select(Some(0));
        }

        Self {
            sessions,
            list_state,
            should_quit: false,
            error_message: None,
            mode: AppMode::Normal,
            search_query: String::new(),
            confirmed_query: String::new(),
            filtered_indices,
            pre_search_selection: None,
            // Searchable text cache is lazily built on first search
            searchable_text_cache: None,
            title_cache,
        }
    }

    /// Reloads sessions from disk.
    /// If changes are provided, only those sessions are reloaded (incremental).
    /// If None, performs a full reload.
    /// Preserves the selection by session_id if possible.
    pub fn reload_sessions(&mut self, changes: Option<&[SessionChange]>) -> Result<()> {
        match changes {
            Some(changes) => self.apply_incremental_changes(changes)?,
            None => self.full_reload()?,
        }
        Ok(())
    }

    /// Performs a full reload of all sessions.
    fn full_reload(&mut self) -> Result<()> {
        // Remember the currently selected session_id
        let selected_session_id = self.selected_session().map(|s| s.session_id.clone());

        self.sessions = load_sessions()?;

        // Rebuild title cache for new/changed sessions
        self.rebuild_title_cache();

        // Incrementally update searchable text cache if it exists
        if self.searchable_text_cache.is_some() {
            self.update_searchable_text_cache();
        }

        // Re-apply filter with current query
        self.apply_filter();
        self.restore_selection(selected_session_id.as_deref());

        Ok(())
    }

    /// Applies incremental changes to the session list.
    fn apply_incremental_changes(&mut self, changes: &[SessionChange]) -> Result<()> {
        let selected_session_id = self.selected_session().map(|s| s.session_id.clone());

        for change in changes {
            match change.change_type {
                SessionChangeType::Created | SessionChangeType::Modified => {
                    // Load the specific session
                    if let Some(session) = store::load_session(&change.session_id)? {
                        // Check if session is stale (TTY check)
                        if is_session_stale(&session) {
                            self.remove_session(&change.session_id);
                            store::delete_session(&change.session_id)?;
                        } else {
                            self.upsert_session(session);
                        }
                    } else {
                        // File was deleted or corrupted
                        self.remove_session(&change.session_id);
                    }
                }
                SessionChangeType::Deleted => {
                    self.remove_session(&change.session_id);
                }
            }
        }

        // Re-sort sessions by updated_at descending
        self.sessions
            .sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        // Rebuild caches for changed sessions only
        self.rebuild_title_cache_incremental(changes);

        if self.searchable_text_cache.is_some() {
            self.update_searchable_text_cache();
        }

        self.apply_filter();
        self.restore_selection(selected_session_id.as_deref());

        Ok(())
    }

    /// Inserts or updates a session in the list.
    fn upsert_session(&mut self, session: Session) {
        if let Some(existing) = self
            .sessions
            .iter_mut()
            .find(|s| s.session_id == session.session_id)
        {
            *existing = session;
        } else {
            self.sessions.push(session);
        }
    }

    /// Removes a session from the list and caches.
    fn remove_session(&mut self, session_id: &str) {
        self.sessions.retain(|s| s.session_id != session_id);
        self.title_cache.remove(session_id);
        if let Some(ref mut cache) = self.searchable_text_cache {
            cache.remove(session_id);
        }
    }

    /// Restores selection by session_id if possible, otherwise adjusts.
    fn restore_selection(&mut self, session_id: Option<&str>) {
        if let Some(id) = session_id
            && let Some(filtered_pos) = self
                .filtered_indices
                .iter()
                .position(|&i| self.sessions.get(i).is_some_and(|s| s.session_id == id))
        {
            self.list_state.select(Some(filtered_pos));
            return;
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
    /// Lazily builds the searchable text cache on first use.
    pub fn enter_search_mode(&mut self) {
        // Build searchable text cache on first search
        if self.searchable_text_cache.is_none() {
            self.searchable_text_cache = Some(build_searchable_text_cache(&self.sessions));
        }

        self.pre_search_selection = self.list_state.selected();
        self.search_query = self.confirmed_query.clone();
        self.mode = AppMode::Search;
    }

    /// Returns the cached title for a session, if available.
    pub fn get_cached_title(&self, session_id: &str) -> Option<&str> {
        self.title_cache.get(session_id).map(String::as_str)
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
        } else if let Some(ref cache) = self.searchable_text_cache {
            self.filtered_indices = self
                .sessions
                .iter()
                .enumerate()
                .filter(|(_, session)| session_matches_cached(session, query, cache))
                .map(|(i, _)| i)
                .collect();
        } else {
            // Cache not built yet, show all sessions
            self.filtered_indices = (0..self.sessions.len()).collect();
        }

        // Reset selection to first item or none
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    /// Incrementally updates the searchable text cache.
    /// Only rebuilds entries for sessions that have been modified since last cache.
    fn update_searchable_text_cache(&mut self) {
        let Some(ref mut cache) = self.searchable_text_cache else {
            return;
        };

        // Remove entries for sessions that no longer exist
        let session_ids: std::collections::HashSet<&str> = self
            .sessions
            .iter()
            .map(|s| s.session_id.as_str())
            .collect();
        cache.retain(|id, _| session_ids.contains(id.as_str()));

        // Update entries for new or modified sessions
        for session in &self.sessions {
            let needs_update = cache
                .get(&session.session_id)
                .is_none_or(|(_, cached_at)| *cached_at < session.updated_at);

            if needs_update {
                let text = build_searchable_text(session);
                cache.insert(session.session_id.clone(), (text, session.updated_at));
            }
        }
    }

    /// Rebuilds the title cache for all sessions.
    fn rebuild_title_cache(&mut self) {
        self.title_cache = build_title_cache(&self.sessions);
    }

    /// Incrementally updates the title cache for changed sessions only.
    fn rebuild_title_cache_incremental(&mut self, changes: &[SessionChange]) {
        for change in changes {
            match change.change_type {
                SessionChangeType::Created | SessionChangeType::Modified => {
                    if let Some(session) = self
                        .sessions
                        .iter()
                        .find(|s| s.session_id == change.session_id)
                    {
                        let title = get_title_display_name(session);
                        self.title_cache.insert(change.session_id.clone(), title);
                    }
                }
                SessionChangeType::Deleted => {
                    self.title_cache.remove(&change.session_id);
                }
            }
        }
    }
}

/// Checks if a session is stale (TTY no longer exists).
fn is_session_stale(session: &Session) -> bool {
    if !tmux::is_server_available() {
        return false;
    }
    session
        .tmux_info
        .as_ref()
        .is_some_and(|info| !tmux::is_pane_alive(&info.pane_id))
}

/// Builds the searchable text cache for all sessions.
fn build_searchable_text_cache(sessions: &[Session]) -> HashMap<String, (String, DateTime<Utc>)> {
    sessions
        .iter()
        .map(|session| {
            let searchable_text = build_searchable_text(session);
            (
                session.session_id.clone(),
                (searchable_text, session.updated_at),
            )
        })
        .collect()
}

/// Builds the title cache for all sessions.
fn build_title_cache(sessions: &[Session]) -> HashMap<String, String> {
    sessions
        .iter()
        .map(|session| {
            let title = get_title_display_name(session);
            (session.session_id.clone(), title)
        })
        .collect()
}

/// Gets the title display name for a session.
/// Fetches from Claude Code's sessions-index.json, falls back to tmux session:window or cwd.
/// All outputs are sanitized to strip ANSI escape sequences.
fn get_title_display_name(session: &Session) -> String {
    if let Some(title) = claude_sessions::get_session_title(&session.cwd, &session.session_id) {
        // Already sanitized by claude_sessions::normalize_title
        return title;
    }

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

/// Checks if a session matches the search query using the cache.
/// Uses case-insensitive partial matching with AND logic for multiple words.
fn session_matches_cached(
    session: &Session,
    query: &str,
    cache: &HashMap<String, (String, DateTime<Utc>)>,
) -> bool {
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return true;
    }

    // Get searchable text from cache, or build it on the fly as fallback
    let searchable = cache
        .get(&session.session_id)
        .map(|(text, _)| text.as_str())
        .unwrap_or("");
    let searchable_lower = searchable.to_lowercase();

    // All words must match (AND logic)
    words
        .iter()
        .all(|word| searchable_lower.contains(&word.to_lowercase()))
}

/// Checks if a session matches the search query (without cache).
/// Used for testing. Builds searchable text on the fly.
#[cfg(test)]
fn session_matches(session: &Session, query: &str) -> bool {
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return true;
    }

    let searchable = build_searchable_text(session);
    let searchable_lower = searchable.to_lowercase();

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

    // All conversation text (user messages and assistant responses, excluding tool outputs)
    if let Some(conversation) =
        claude_sessions::get_conversation_text(&session.cwd, &session.session_id)
    {
        parts.push(conversation);
    } else if let Some(ref msg) = session.last_message {
        // Fallback to last_message if transcript is not available
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
    use rstest::rstest;
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
        App::with_sessions(sessions)
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

    #[rstest]
    #[case::valid_number(2, Some(0), Some(1))]
    #[case::out_of_range(10, Some(1), Some(1))]
    #[case::zero_ignored(0, Some(1), Some(1))]
    fn test_select_by_number(
        #[case] num: usize,
        #[case] initial: Option<usize>,
        #[case] expected: Option<usize>,
    ) {
        let mut app = create_test_app(vec![
            create_test_session("1"),
            create_test_session("2"),
            create_test_session("3"),
        ]);
        app.list_state.select(initial);

        app.select_by_number(num);
        assert_eq!(app.list_state.selected(), expected);
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

    #[rstest]
    #[case::empty("", true)]
    #[case::whitespace("   ", true)]
    fn test_session_matches_empty_query(#[case] query: &str, #[case] expected: bool) {
        let session = create_test_session("test");
        assert_eq!(session_matches(&session, query), expected);
    }

    #[rstest]
    #[case::exact_match("project", true)]
    #[case::case_insensitive("PROJECT", true)]
    #[case::parent_dir("user", true)]
    #[case::nonexistent("nonexistent", false)]
    fn test_session_matches_cwd(#[case] query: &str, #[case] expected: bool) {
        let mut session = create_test_session("test");
        session.cwd = PathBuf::from("/home/user/project");
        assert_eq!(session_matches(&session, query), expected);
    }

    #[rstest]
    #[case::session_name("webapp", true)]
    #[case::window_name("editor", true)]
    #[case::case_insensitive("WEBAPP", true)]
    #[case::nonexistent("nonexistent", false)]
    fn test_session_matches_tmux_info(#[case] query: &str, #[case] expected: bool) {
        let mut session = create_test_session("test");
        session.tmux_info = Some(TmuxInfo {
            session_name: "webapp".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        assert_eq!(session_matches(&session, query), expected);
    }

    #[rstest]
    #[case::word_in_message("updated", true)]
    #[case::another_word("code", true)]
    #[case::nonexistent("nonexistent", false)]
    fn test_session_matches_last_message(#[case] query: &str, #[case] expected: bool) {
        let mut session = create_test_session("test");
        session.last_message = Some("I've updated the code".to_string());
        assert_eq!(session_matches(&session, query), expected);
    }

    #[rstest]
    #[case::both_match("webapp feature", true)]
    #[case::across_fields("user working", true)]
    #[case::one_missing("webapp nonexistent", false)]
    fn test_session_matches_and_logic(#[case] query: &str, #[case] expected: bool) {
        let mut session = create_test_session("test");
        session.cwd = PathBuf::from("/home/user/webapp");
        session.last_message = Some("Working on feature".to_string());
        assert_eq!(session_matches(&session, query), expected);
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
        assert_eq!(app.filtered_indices, vec![0]);
        assert!(app.has_filter());
    }

    #[test]
    fn test_cancel_search() {
        let mut session1 = create_test_session("1");
        session1.cwd = PathBuf::from("/home/user/webapp");
        let mut session2 = create_test_session("2");
        session2.cwd = PathBuf::from("/home/user/api");

        let mut app = create_test_app(vec![session1, session2]);
        app.list_state.select(Some(1));
        app.enter_search_mode();
        app.update_search_query("webapp".to_string());

        assert_eq!(app.filtered_indices, vec![0]);

        app.cancel_search();

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

        assert_eq!(app.filtered_indices, vec![0, 2]);
        assert_eq!(app.list_state.selected(), Some(0));

        app.select_next();
        assert_eq!(app.list_state.selected(), Some(1));
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("3")
        );

        app.select_next();
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[rstest]
    #[case::select_second(2, "3")]
    fn test_select_by_number_with_filter(#[case] num: usize, #[case] expected_id: &str) {
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

        app.select_by_number(num);
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some(expected_id)
        );
    }

    #[test]
    fn test_select_by_number_out_of_range_with_filter() {
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

        app.select_by_number(2);
        app.select_by_number(3); // Out of range
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("3")
        );
    }
}
