use std::collections::HashMap;

use crate::commands::cc::types::Session;

use super::super::session_rows::nearest_living_ancestor;
use super::App;

impl App {
    /// Moves selection to the next selectable row in the displayed list,
    /// wrapping around. Skips section headers.
    pub fn select_next(&mut self) {
        let selectable = self.selectable_positions();
        if selectable.is_empty() {
            return;
        }

        let current = self.list_state.selected();
        let next = match current.and_then(|pos| selectable.iter().position(|&i| i == pos)) {
            Some(idx) => selectable[(idx + 1) % selectable.len()],
            None => selectable[0],
        };
        self.list_state.select(Some(next));
        self.persist_selection();
    }

    /// Moves selection to the previous selectable row in the displayed
    /// list, wrapping around. Skips section headers.
    pub fn select_previous(&mut self) {
        let selectable = self.selectable_positions();
        if selectable.is_empty() {
            return;
        }

        let current = self.list_state.selected();
        let next = match current.and_then(|pos| selectable.iter().position(|&i| i == pos)) {
            Some(idx) => selectable[(idx + selectable.len() - 1) % selectable.len()],
            None => selectable[0],
        };
        self.list_state.select(Some(next));
        self.persist_selection();
    }

    /// Selects a session by its 1-indexed number (1-9) among the
    /// selectable rows of the displayed list.
    pub fn select_by_number(&mut self, num: usize) {
        if num == 0 {
            return;
        }
        let selectable = self.selectable_positions();
        if let Some(&pos) = selectable.get(num - 1) {
            self.list_state.select(Some(pos));
            self.persist_selection();
        }
    }

    /// Moves the cursor to the selected session's nearest displayed
    /// ancestor -- the same session named in its breadcrumb prefix (see
    /// `session_rows::nearest_living_ancestor`), so the jump target and the
    /// breadcrumb never disagree.
    ///
    /// No-op for a root session (no ancestors at all). If the session has
    /// ancestors but none of them are currently displayed (filtered out by
    /// search or a status filter), sets an error rather than doing nothing
    /// silently, since a silent no-op would be indistinguishable from the
    /// root case.
    pub fn select_parent(&mut self) {
        let Some(session) = self.selected_session() else {
            return;
        };
        if session.ancestor_session_ids.is_empty() {
            return;
        }

        let filtered = self.filtered_sessions();
        let by_id: HashMap<&str, &Session> = filtered
            .iter()
            .map(|s| (s.session_id.as_str(), *s))
            .collect();
        let ancestor_id =
            nearest_living_ancestor(session, &by_id).map(|ancestor| ancestor.session_id.clone());

        match ancestor_id {
            Some(id) => {
                if let Some(pos) = self.position_of_session_row(&id) {
                    self.list_state.select(Some(pos));
                    self.persist_selection();
                }
            }
            None => {
                self.set_error("Parent session is filtered out of the current view".to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::SessionStatus;
    use chrono::{TimeDelta, Utc};
    use rstest::rstest;
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
            read_at: None,
            sweep_signaled: false,
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
        // Both sessions default to `Running`, so row 0 is the "RUNNING (2)"
        // header (not selectable) and rows 1-2 are the sessions. Starting
        // on the last selectable row exercises wraparound skipping the
        // header at row 0 rather than landing on it.
        app.list_state.select(Some(2));

        app.select_next();
        assert_eq!(app.list_state.selected(), Some(1));
    }

    #[test]
    fn test_select_previous_wraps() {
        let mut app = create_test_app(vec![create_test_session("1"), create_test_session("2")]);
        // See test_select_next_wraps: row 0 is a header, rows 1-2 are the
        // sessions. Starting on the first selectable row exercises
        // wraparound skipping the header straight to the last row.
        app.list_state.select(Some(1));

        app.select_previous();
        assert_eq!(app.list_state.selected(), Some(2));
    }

    // 3 sessions default to `Running`, so row 0 is the "RUNNING (3)" header
    // and rows 1-3 are the sessions; `select_by_number` picks among rows
    // 1-3 regardless of `initial` (it does not depend on current selection).
    #[rstest]
    #[case::valid_number(2, Some(1), Some(2))]
    #[case::out_of_range(10, Some(2), Some(2))]
    #[case::zero_ignored(0, Some(2), Some(2))]
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

    // =========================================================================
    // select_parent tests
    // =========================================================================

    /// (selected session id, error message) pair, for asserting the full
    /// effect of `select_parent` with one equality check. Owned (not
    /// borrowed) so callers can keep mutating `app` afterward.
    fn parent_selection_state(app: &App) -> (Option<String>, Option<String>) {
        (
            app.selected_session().map(|s| s.session_id.clone()),
            app.error_message.clone(),
        )
    }

    #[test]
    fn test_select_parent_root_session_is_noop() {
        let mut app = create_test_app(vec![create_test_session("root")]);
        app.select_by_number(1);

        app.select_parent();

        assert_eq!(
            parent_selection_state(&app),
            (Some("root".to_string()), None)
        );
    }

    #[test]
    fn test_select_parent_moves_to_displayed_ancestor() {
        let root = create_test_session("root");
        let mut child = create_test_session("child");
        child.ancestor_session_ids = vec!["root".to_string()];

        // Row 0 is the "RUNNING (2)" header; "root"/"child" are rows 1/2.
        let mut app = create_test_app(vec![root, child]);
        app.select_by_number(2);

        app.select_parent();

        assert_eq!(
            parent_selection_state(&app),
            (Some("root".to_string()), None)
        );
    }

    #[test]
    fn test_select_parent_repeated_walks_up_to_root() {
        let root = create_test_session("root");
        let mut parent = create_test_session("parent");
        parent.ancestor_session_ids = vec!["root".to_string()];
        let mut child = create_test_session("child");
        child.ancestor_session_ids = vec!["root".to_string(), "parent".to_string()];

        // Row 0 is the "RUNNING (3)" header; rows 1-3 are root/parent/child.
        let mut app = create_test_app(vec![root, parent, child]);
        app.select_by_number(3);

        app.select_parent();
        let after_first_hop = parent_selection_state(&app);
        app.select_parent();
        let after_second_hop = parent_selection_state(&app);

        assert_eq!(
            [after_first_hop, after_second_hop],
            [
                (Some("parent".to_string()), None),
                (Some("root".to_string()), None)
            ]
        );
    }

    #[test]
    fn test_select_parent_ancestor_filtered_out_sets_error() {
        let mut root = create_test_session("root");
        root.status = SessionStatus::Paused;
        let mut child = create_test_session("child");
        child.ancestor_session_ids = vec!["root".to_string()];

        let mut app = create_test_app(vec![root, child]);
        // Filtering to Running hides the Paused root, leaving "child" as the
        // only (and already selected) row.
        app.toggle_status_filter(SessionStatus::Running);

        app.select_parent();

        assert_eq!(
            parent_selection_state(&app),
            (
                Some("child".to_string()),
                Some("Parent session is filtered out of the current view".to_string())
            )
        );
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
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("1")
        );

        app.select_next();
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("3")
        );

        app.select_next();
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("1")
        );
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
