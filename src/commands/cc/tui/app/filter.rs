use crate::commands::cc::claude_sessions;
use crate::commands::cc::types::{Session, SessionStatus};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

use super::super::session_rows::is_descendant_of;
use super::{App, AppMode};

impl App {
    /// Returns whether a filter is currently active.
    pub fn has_filter(&self) -> bool {
        !self.confirmed_query.is_empty()
            || self.status_filter.is_some()
            || self.drilldown_scope.is_some()
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

    /// Exits search mode, confirming the search.
    /// Preserves the current selection (by session id) when possible.
    pub fn confirm_search(&mut self) {
        let old_pos = self.list_state.selected();
        let old_id = self.selected_session().map(|s| s.session_id.clone());
        self.confirmed_query = self.search_query.clone();
        self.apply_filter();
        self.resync_selection(old_pos, old_id);
        self.mode = AppMode::Normal;
        self.pre_search_selection = None;
    }

    /// Exits search mode, cancelling the search.
    pub fn cancel_search(&mut self) {
        let old_pos = self.list_state.selected();
        let old_id = self.selected_session().map(|s| s.session_id.clone());
        self.search_query = self.confirmed_query.clone();
        self.apply_filter();
        self.resync_selection(old_pos, old_id);
        self.mode = AppMode::Normal;
        self.pre_search_selection = None;
    }

    /// Clears the filter and shows all sessions.
    pub fn clear_filter(&mut self) {
        self.search_query.clear();
        self.confirmed_query.clear();
        self.status_filter = None;
        self.drilldown_scope = None;
        self.filtered_indices = (0..self.sessions.len()).collect();
        self.rebuild_row_order();
        self.list_state
            .select(self.selectable_positions().first().copied());
    }

    /// Toggles a status filter. If the same status is already active, clears it.
    pub fn toggle_status_filter(&mut self, status: SessionStatus) {
        if self.status_filter == Some(status) {
            self.status_filter = None;
        } else {
            self.status_filter = Some(status);
        }
        self.apply_filter();
    }

    /// Updates the search query and re-applies the filter.
    pub fn update_search_query(&mut self, query: String) {
        self.search_query = query;
        self.apply_filter();
    }

    /// Applies the current search query, status filter, and drill-down scope
    /// (all AND'd together) to filter sessions.
    pub(super) fn apply_filter(&mut self) {
        // A scoped-out root (deleted, or dropped by a reload) leaves the
        // scope with nothing to anchor on -- clear it rather than filtering
        // everything else out along with it, which would strand the user
        // looking at an empty list with no visible way back.
        if let Some(root_id) = &self.drilldown_scope
            && !self.sessions.iter().any(|s| &s.session_id == root_id)
        {
            self.drilldown_scope = None;
        }

        let query = if self.mode == AppMode::Search {
            &self.search_query
        } else {
            &self.confirmed_query
        };

        let status_filter = self.status_filter;
        let scope_root = self.drilldown_scope.clone();

        self.filtered_indices = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| {
                // Drill-down scope (AND with status/text filters below)
                if let Some(ref root_id) = scope_root
                    && session.session_id != *root_id
                    && !is_descendant_of(session, root_id)
                {
                    return false;
                }

                // Status filter (AND with text search)
                if let Some(status) = status_filter
                    && session.status != status
                {
                    return false;
                }

                // Text search filter
                if !query.is_empty()
                    && let Some(ref cache) = self.searchable_text_cache
                {
                    return session_matches_cached(session, query, cache);
                }

                true
            })
            .map(|(i, _)| i)
            .collect();

        self.rebuild_row_order();

        // Reset selection to the first selectable row, or none.
        self.list_state
            .select(self.selectable_positions().first().copied());
    }

    /// Incrementally updates the searchable text cache.
    /// Only rebuilds entries for sessions that have been modified since last cache.
    pub(super) fn update_searchable_text_cache(&mut self) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::TmuxInfo;
    use chrono::TimeDelta;
    use rstest::{fixture, rstest};
    use std::path::PathBuf;

    /// Counter to assign distinct timestamps to test sessions.
    /// Each call returns a progressively older timestamp, so sessions
    /// created first sort first (most recent updated_at).
    use std::sync::atomic::{AtomicI64, Ordering};
    static TEST_SESSION_COUNTER: AtomicI64 = AtomicI64::new(0);

    fn create_test_session(id: &str) -> Session {
        let offset = TEST_SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let now = Utc::now();
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: now - TimeDelta::seconds(offset),
            updated_at: now - TimeDelta::seconds(offset),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            pending_agent_task_ids: std::collections::BTreeSet::new(),
            pending_permission_agent_ids: std::collections::BTreeSet::new(),
            read_at: None,
            sweep_signaled: false,
        }
    }

    fn create_test_app(sessions: Vec<Session>) -> App {
        App::with_sessions(sessions)
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
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("1")
        );
    }

    // =========================================================================
    // Status filter tests
    // =========================================================================

    /// Helper to create a session with a specific status.
    fn create_session_with_status(id: &str, status: SessionStatus) -> Session {
        let mut session = create_test_session(id);
        session.status = status;
        session
    }

    #[fixture]
    fn app_with_mixed_statuses() -> App {
        create_test_app(vec![
            create_session_with_status("running-1", SessionStatus::Running),
            create_session_with_status("waiting-1", SessionStatus::WaitingInput),
            create_session_with_status("stopped-1", SessionStatus::Stopped),
            create_session_with_status("waiting-2", SessionStatus::WaitingInput),
        ])
    }

    #[rstest]
    #[case::waiting_filter(
        SessionStatus::WaitingInput,
        vec!["waiting-1", "waiting-2"]
    )]
    #[case::stopped_filter(
        SessionStatus::Stopped,
        vec!["stopped-1"]
    )]
    #[case::running_filter(
        SessionStatus::Running,
        vec!["running-1"]
    )]
    fn test_toggle_status_filter(
        mut app_with_mixed_statuses: App,
        #[case] status: SessionStatus,
        #[case] expected_ids: Vec<&str>,
    ) {
        app_with_mixed_statuses.toggle_status_filter(status);

        let filtered: Vec<&str> = app_with_mixed_statuses
            .filtered_sessions()
            .iter()
            .map(|s| s.session_id.as_str())
            .collect();
        assert_eq!(filtered, expected_ids);
    }

    #[rstest]
    fn test_toggle_status_filter_off(mut app_with_mixed_statuses: App) {
        // Toggle on
        app_with_mixed_statuses.toggle_status_filter(SessionStatus::WaitingInput);
        assert_eq!(app_with_mixed_statuses.filtered_sessions().len(), 2);

        // Toggle off (same status again)
        app_with_mixed_statuses.toggle_status_filter(SessionStatus::WaitingInput);
        assert!(app_with_mixed_statuses.status_filter.is_none());
        assert_eq!(app_with_mixed_statuses.filtered_sessions().len(), 4);
    }

    #[test]
    fn test_status_filter_with_text_search() {
        let mut session_running = create_session_with_status("running-1", SessionStatus::Running);
        session_running.cwd = PathBuf::from("/home/user/webapp");

        let mut session_waiting =
            create_session_with_status("waiting-1", SessionStatus::WaitingInput);
        session_waiting.cwd = PathBuf::from("/home/user/webapp");

        let mut session_other =
            create_session_with_status("waiting-2", SessionStatus::WaitingInput);
        session_other.cwd = PathBuf::from("/home/user/api");

        let mut app = create_test_app(vec![session_running, session_waiting, session_other]);

        // Set status filter to WaitingInput
        app.toggle_status_filter(SessionStatus::WaitingInput);

        // Enter search mode and search for "webapp"
        app.enter_search_mode();
        app.update_search_query("webapp".to_string());
        app.confirm_search();

        // Only the WaitingInput session with "webapp" in cwd should match (AND logic)
        let filtered: Vec<&str> = app
            .filtered_sessions()
            .iter()
            .map(|s| s.session_id.as_str())
            .collect();
        assert_eq!(filtered, vec!["waiting-1"]);
    }

    #[rstest]
    fn test_has_filter_with_status_only(mut app_with_mixed_statuses: App) {
        assert!(!app_with_mixed_statuses.has_filter());

        app_with_mixed_statuses.toggle_status_filter(SessionStatus::Running);
        assert!(app_with_mixed_statuses.has_filter());
    }

    #[rstest]
    fn test_clear_filter_clears_status(mut app_with_mixed_statuses: App) {
        app_with_mixed_statuses.toggle_status_filter(SessionStatus::Stopped);
        assert_eq!(app_with_mixed_statuses.filtered_sessions().len(), 1);

        app_with_mixed_statuses.clear_filter();

        assert!(app_with_mixed_statuses.status_filter.is_none());
        assert!(!app_with_mixed_statuses.has_filter());
        assert_eq!(app_with_mixed_statuses.filtered_sessions().len(), 4);
    }

    // =========================================================================
    // Drill-down scope tests
    // =========================================================================

    #[test]
    fn test_has_filter_with_drilldown_scope_only() {
        let root = create_test_session("root");
        let mut leaf = create_test_session("leaf");
        leaf.ancestor_session_ids = vec!["root".to_string()];

        // "root" is selected by default (first selectable row).
        let mut app = create_test_app(vec![root, leaf]);
        assert!(!app.has_filter());

        app.enter_drilldown();

        assert!(app.has_filter());
    }

    #[test]
    fn test_clear_filter_clears_drilldown_scope() {
        let root = create_test_session("root");
        let mut leaf = create_test_session("leaf");
        leaf.ancestor_session_ids = vec!["root".to_string()];

        let mut app = create_test_app(vec![root, leaf]);
        app.enter_drilldown();
        assert!(app.has_filter());

        app.clear_filter();

        assert_eq!(app.drilldown_scope, None);
        assert!(!app.has_filter());
    }

    #[test]
    fn test_drilldown_scope_and_status_filter_intersect() {
        let root = create_test_session("root");
        let mut running_child = create_test_session("running-child");
        running_child.ancestor_session_ids = vec!["root".to_string()];
        let mut paused_child = create_session_with_status("paused-child", SessionStatus::Paused);
        paused_child.ancestor_session_ids = vec!["root".to_string()];

        let mut app = create_test_app(vec![root, running_child, paused_child]);
        app.enter_drilldown();

        app.toggle_status_filter(SessionStatus::Running);

        assert_eq!(
            app.filtered_sessions()
                .iter()
                .map(|s| s.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["root", "running-child"]
        );
    }

    #[test]
    fn test_drilldown_scope_and_text_search_intersect() {
        let mut root = create_test_session("root");
        root.cwd = PathBuf::from("/home/user/webapp");
        let mut matching_child = create_test_session("matching-child");
        matching_child.cwd = PathBuf::from("/home/user/webapp/api");
        matching_child.ancestor_session_ids = vec!["root".to_string()];
        let mut other_child = create_test_session("other-child");
        other_child.cwd = PathBuf::from("/home/user/other");
        other_child.ancestor_session_ids = vec!["root".to_string()];

        let mut app = create_test_app(vec![root, matching_child, other_child]);
        app.enter_drilldown();

        app.enter_search_mode();
        app.update_search_query("webapp".to_string());
        app.confirm_search();

        assert_eq!(
            app.filtered_sessions()
                .iter()
                .map(|s| s.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["root", "matching-child"]
        );
    }

    #[test]
    fn test_apply_filter_clears_drilldown_scope_when_root_removed() {
        let root = create_test_session("root");
        let mut leaf = create_test_session("leaf");
        leaf.ancestor_session_ids = vec!["root".to_string()];

        let mut app = create_test_app(vec![root, leaf]);
        app.enter_drilldown();
        assert_eq!(app.drilldown_scope, Some("root".to_string()));

        // Simulate a reload that dropped "root" (e.g. the session ended and
        // was pruned) -- mirrors how `app/reload.rs`'s own tests mutate
        // `app.sessions` directly before re-applying the filter.
        app.sessions.retain(|s| s.session_id != "root");
        app.apply_filter();

        assert_eq!(app.drilldown_scope, None);
        assert_eq!(
            app.filtered_sessions()
                .iter()
                .map(|s| s.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["leaf"]
        );
    }
}
