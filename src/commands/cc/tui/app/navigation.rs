use std::collections::HashMap;

use crate::commands::cc::types::Session;

use super::super::session_rows::{is_descendant_of, nearest_living_ancestor};
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
    /// root case -- unless the selected session is the drill-down scope's
    /// root, in which case "no displayed ancestor" instead exits the scope
    /// (see [`Self::is_at_drilldown_root`]).
    pub fn select_parent(&mut self) {
        let Some(session) = self.selected_session() else {
            return;
        };
        if session.ancestor_session_ids.is_empty() {
            if self.is_at_drilldown_root() {
                self.exit_drilldown();
            }
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
                if self.is_at_drilldown_root() {
                    self.exit_drilldown();
                } else {
                    self.set_error(
                        "Parent session is filtered out of the current view".to_string(),
                    );
                }
            }
        }
    }

    /// Whether the currently selected session is exactly the drill-down
    /// scope's root. This is the only case where "no displayed ancestor"
    /// can occur while scoped -- every other displayed session's nearest
    /// ancestor is either another scoped descendant or the root itself.
    fn is_at_drilldown_root(&self) -> bool {
        let Some(root_id) = self.drilldown_scope.as_deref() else {
            return false;
        };
        self.selected_session()
            .is_some_and(|s| s.session_id == root_id)
    }

    /// Enters drill-down scope on the selected session: the list narrows to
    /// that session plus its descendants (any depth, matching the `▸{n}`
    /// badge rule -- see `session_rows::is_descendant_of`). No-op when the
    /// session has no descendants (i.e. no badge is shown for it), and when
    /// there is no selection. Keeps the cursor on the same session --
    /// jumping the selection elsewhere would make the scope change hard to
    /// follow.
    pub fn enter_drilldown(&mut self) {
        let Some(session) = self.selected_session() else {
            return;
        };
        let session_id = session.session_id.clone();
        // Check against the currently displayed sessions, not all of
        // `self.sessions` -- must match exactly what `descendant_counts`
        // (the `▸{n}` badge) considers, or a session hidden by search/status
        // filtering could enter a scope for a badge the user can't see.
        let has_descendants = self
            .filtered_sessions()
            .iter()
            .any(|s| is_descendant_of(s, &session_id));
        if !has_descendants {
            return;
        }

        self.drilldown_scope = Some(session_id.clone());
        self.apply_filter();
        if let Some(pos) = self.position_of_session_row(&session_id) {
            self.list_state.select(Some(pos));
        }
    }

    /// Exits drill-down scope, restoring the search/status-filtered list.
    /// Keeps the cursor on the same session (the former scope root), which
    /// remains selectable in the exited list.
    pub fn exit_drilldown(&mut self) {
        let old_id = self.selected_session().map(|s| s.session_id.clone());
        self.drilldown_scope = None;
        self.apply_filter();
        if let Some(id) = old_id
            && let Some(pos) = self.position_of_session_row(&id)
        {
            self.list_state.select(Some(pos));
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
            pending_permission_agent_ids: std::collections::BTreeSet::new(),
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

    /// (selected session id, drill-down scope, error message) triple, for
    /// asserting the combined effect of drill-down navigation with one
    /// equality check.
    fn scoped_selection_state(app: &App) -> (Option<String>, Option<String>, Option<String>) {
        (
            app.selected_session().map(|s| s.session_id.clone()),
            app.drilldown_scope.clone(),
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

    // =========================================================================
    // Drill-down tests
    // =========================================================================

    #[test]
    fn test_enter_drilldown_narrows_to_root_and_descendants() {
        let root = create_test_session("root");
        let mut mid = create_test_session("mid");
        mid.ancestor_session_ids = vec!["root".to_string()];
        let mut leaf = create_test_session("leaf");
        leaf.ancestor_session_ids = vec!["root".to_string(), "mid".to_string()];

        // Row 0 is the "RUNNING (3)" header; rows 1-3 are root/mid/leaf.
        let mut app = create_test_app(vec![root, mid, leaf]);
        app.select_by_number(1);

        app.enter_drilldown();

        assert_eq!(
            app.filtered_sessions()
                .iter()
                .map(|s| s.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["root", "mid", "leaf"]
        );
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("root")
        );
    }

    #[test]
    fn test_enter_drilldown_without_descendants_is_noop() {
        let mut app = create_test_app(vec![create_test_session("solo")]);
        app.select_by_number(1);
        let filtered_before = app.filtered_indices.clone();

        app.enter_drilldown();

        assert_eq!(app.drilldown_scope, None);
        assert_eq!(app.filtered_indices, filtered_before);
    }

    #[test]
    fn test_enter_drilldown_without_selection_is_noop() {
        let mut app = create_test_app(vec![]);

        app.enter_drilldown();

        assert_eq!(app.drilldown_scope, None);
    }

    #[test]
    fn test_enter_drilldown_noop_when_only_descendant_is_hidden_by_filter() {
        // "root"'s only descendant ("child") does not match the active
        // status filter, so no badge is shown for "root" -- entering
        // drilldown must agree with that and stay a no-op, not scope into a
        // session the user has no visible reason to believe has children.
        let root = create_test_session("root");
        let mut child = create_test_session("child");
        child.status = SessionStatus::Paused;
        child.ancestor_session_ids = vec!["root".to_string()];

        let mut app = create_test_app(vec![root, child]);
        app.toggle_status_filter(SessionStatus::Running);
        app.select_by_number(1);

        app.enter_drilldown();

        assert_eq!(app.drilldown_scope, None);
    }

    /// A true root (no ancestors at all) with one descendant and one
    /// unrelated top-level session, so exiting the scope has something
    /// observable to widen back into.
    fn sessions_root_with_no_ancestors() -> Vec<Session> {
        let root = create_test_session("root");
        let mut leaf = create_test_session("leaf");
        leaf.ancestor_session_ids = vec!["root".to_string()];
        let other = create_test_session("other");
        vec![root, leaf, other]
    }

    /// A scope root whose real ancestor ("grandparent") exists in
    /// `app.sessions` but sits outside the scope's own descendant chain, so
    /// it's excluded from the filtered view by the scope boundary itself
    /// rather than by deletion.
    fn sessions_root_with_ancestors_outside_scope() -> Vec<Session> {
        let grandparent = create_test_session("grandparent");
        let mut root = create_test_session("root");
        root.ancestor_session_ids = vec!["grandparent".to_string()];
        let mut leaf = create_test_session("leaf");
        leaf.ancestor_session_ids = vec!["grandparent".to_string(), "root".to_string()];
        vec![grandparent, root, leaf]
    }

    #[rstest]
    #[case::no_ancestors(sessions_root_with_no_ancestors(), vec!["root", "leaf", "other"])]
    #[case::ancestors_outside_scope(
        sessions_root_with_ancestors_outside_scope(),
        vec!["grandparent", "root", "leaf"]
    )]
    fn test_select_parent_at_drilldown_root_with_nothing_to_jump_to_exits_scope(
        #[case] sessions: Vec<Session>,
        #[case] expected_filtered_after_exit: Vec<&str>,
    ) {
        let mut app = create_test_app(sessions);
        app.list_state.select(app.position_of_session_row("root"));
        app.enter_drilldown();

        app.select_parent();

        assert_eq!(
            scoped_selection_state(&app),
            (Some("root".to_string()), None, None)
        );
        assert_eq!(
            app.filtered_sessions()
                .iter()
                .map(|s| s.session_id.as_str())
                .collect::<Vec<_>>(),
            expected_filtered_after_exit
        );
    }

    #[test]
    fn test_select_parent_repeated_within_scope_only_last_hop_exits() {
        let root = create_test_session("root");
        let mut parent = create_test_session("parent");
        parent.ancestor_session_ids = vec!["root".to_string()];
        let mut child = create_test_session("child");
        child.ancestor_session_ids = vec!["root".to_string(), "parent".to_string()];

        // Row 0 is the "RUNNING (3)" header; rows 1-3 are root/parent/child.
        let mut app = create_test_app(vec![root, parent, child]);
        app.select_by_number(1);
        app.enter_drilldown();
        app.select_by_number(3); // child

        app.select_parent();
        let after_first_hop = scoped_selection_state(&app);
        app.select_parent();
        let after_second_hop = scoped_selection_state(&app);
        app.select_parent();
        let after_third_hop = scoped_selection_state(&app);

        assert_eq!(
            [after_first_hop, after_second_hop, after_third_hop],
            [
                (Some("parent".to_string()), Some("root".to_string()), None),
                (Some("root".to_string()), Some("root".to_string()), None),
                (Some("root".to_string()), None, None),
            ]
        );
    }

    #[test]
    fn test_select_parent_immediate_ancestor_hidden_by_status_filter_sets_error_not_exit() {
        // Regression guard for `is_at_drilldown_root`: the selected session
        // ("leaf") is a scoped descendant, not the scope root itself, so an
        // unrelated status filter hiding its immediate ancestor must still
        // report the existing "filtered out" error rather than exiting the
        // scope.
        let mut root = create_test_session("root");
        root.status = SessionStatus::Paused;
        let mut mid = create_test_session("mid");
        mid.status = SessionStatus::Paused;
        mid.ancestor_session_ids = vec!["root".to_string()];
        let mut leaf = create_test_session("leaf");
        leaf.ancestor_session_ids = vec!["root".to_string(), "mid".to_string()];

        let mut app = create_test_app(vec![root, mid, leaf]);
        app.list_state.select(app.position_of_session_row("root"));
        app.enter_drilldown();
        // Narrows to Running only: excludes root and mid by status (both
        // remain part of the scope's descendant chain), leaving "leaf" as
        // the sole visible row.
        app.toggle_status_filter(SessionStatus::Running);

        app.select_parent();

        assert_eq!(
            scoped_selection_state(&app),
            (
                Some("leaf".to_string()),
                Some("root".to_string()),
                Some("Parent session is filtered out of the current view".to_string())
            )
        );
    }
}
