use crate::commands::agent::claude_sessions;
use crate::commands::agent::session_status;
use crate::commands::agent::store;
#[cfg(test)]
use crate::commands::agent::types::Engine;
use crate::commands::agent::types::{Session, SessionStatus};
use crate::infra::tmux;
use anyhow::Result;
use std::collections::{HashMap, HashSet};

use super::super::event::{SessionChange, SessionChangeType};
use super::super::session_rows::build_session_rows_with_vibe;
use super::App;

#[derive(Debug)]
pub(super) struct SelectionAnchor {
    position: Option<usize>,
    session_id: Option<String>,
    occurrence: usize,
}

impl SelectionAnchor {
    pub(super) fn for_session(session_id: impl Into<String>) -> Self {
        Self {
            position: None,
            session_id: Some(session_id.into()),
            occurrence: 0,
        }
    }
}

impl App {
    /// Reloads sessions from disk.
    /// If changes are provided, only those sessions are reloaded (incremental).
    /// If None, performs a full reload.
    /// Preserves the selected session row when it remains visible.
    pub fn reload_sessions(&mut self, changes: Option<&[SessionChange]>) -> Result<()> {
        match changes {
            Some(changes) => self.apply_incremental_changes(changes)?,
            None => self.full_reload()?,
        }
        Ok(())
    }

    /// Performs a full reload of all sessions.
    fn full_reload(&mut self) -> Result<()> {
        let selection = self.selection_anchor();

        self.sessions = load_sessions()?;
        self.vibe_enabled_session_ids = load_vibe_enabled_session_ids()?;

        // Rebuild title cache for new/changed sessions
        self.rebuild_title_cache();

        // Incrementally update searchable text cache if it exists
        if self.searchable_text_cache.is_some() {
            self.update_searchable_text_cache();
        }

        // Re-apply filter with current query
        self.apply_filter();
        self.restore_selection(selection);

        Ok(())
    }

    /// Applies incremental changes to the session list.
    fn apply_incremental_changes(&mut self, changes: &[SessionChange]) -> Result<()> {
        let selection = self.selection_anchor();

        for change in changes {
            match change.change_type {
                SessionChangeType::Created | SessionChangeType::Modified => {
                    // Load the specific session
                    if let Some(session) =
                        session_status::load_session_with_bg_run_status(&change.session_id)?
                    {
                        if session.vibe_enabled {
                            self.vibe_enabled_session_ids
                                .insert(session.session_id.clone());
                        } else {
                            self.vibe_enabled_session_ids.remove(&session.session_id);
                        }

                        // Check if session is stale (TTY check)
                        if is_session_stale(&session) {
                            self.remove_session(&change.session_id);
                            store::delete_session(&change.session_id)?;
                            self.vibe_enabled_session_ids.remove(&change.session_id);
                        } else if session.status == SessionStatus::Ended {
                            // Ended sessions are not displayed; remove from list
                            self.remove_session(&change.session_id);
                        } else {
                            self.upsert_session(session);
                        }
                    } else {
                        // File was deleted or corrupted
                        self.remove_session(&change.session_id);
                        self.vibe_enabled_session_ids.remove(&change.session_id);
                    }
                }
                SessionChangeType::Deleted => {
                    self.remove_session(&change.session_id);
                    self.vibe_enabled_session_ids.remove(&change.session_id);
                }
            }
        }

        // Re-sort with stability threshold to prevent rapid reordering
        store::sort_sessions(&mut self.sessions);

        // Rebuild caches for changed sessions only
        self.rebuild_title_cache_incremental(changes);

        if self.searchable_text_cache.is_some() {
            self.update_searchable_text_cache();
        }

        self.apply_filter();
        self.restore_selection(selection);

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
    pub(super) fn remove_session(&mut self, session_id: &str) {
        self.sessions.retain(|s| s.session_id != session_id);
        self.title_cache.remove(session_id);
        if let Some(ref mut cache) = self.searchable_text_cache {
            cache.remove(session_id);
        }
    }

    /// Rebuilds `row_sessions` from the current `filtered_indices`.
    ///
    /// Runs the same section-grouping logic that the render layer uses, so
    /// that cursor positions always match the displayed order.
    pub(super) fn rebuild_row_order(&mut self) {
        let filtered: Vec<&Session> = self
            .filtered_indices
            .iter()
            .filter_map(|&i| self.sessions.get(i))
            .collect();
        let rows = build_session_rows_with_vibe(
            &filtered,
            &self.task_by_session,
            &self.agent_config,
            &self.vibe_enabled_session_ids,
        );
        self.row_sessions = rows
            .iter()
            .map(|r| {
                r.session_id()
                    .and_then(|id| self.sessions.iter().position(|s| s.session_id == id))
            })
            .collect();
    }

    /// Indices of rows that are individually selectable (i.e. hold a
    /// session, as opposed to a section header).
    pub(super) fn selectable_positions(&self) -> Vec<usize> {
        self.row_sessions
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.is_some().then_some(i))
            .collect()
    }

    /// The row index whose session has the given id, if that session is
    /// currently displayed.
    pub(super) fn position_of_session_row(&self, session_id: &str) -> Option<usize> {
        self.position_of_session_row_where(|session| session.session_id == session_id)
    }

    fn position_of_session_row_occurrence(
        &self,
        session_id: &str,
        occurrence: usize,
    ) -> Option<usize> {
        self.session_row_positions_where(|session| session.session_id == session_id)
            .nth(occurrence)
            .or_else(|| self.position_of_session_row(session_id))
    }

    fn session_row_positions_where<'a>(
        &'a self,
        predicate: impl Fn(&Session) -> bool + 'a,
    ) -> impl Iterator<Item = usize> + 'a {
        self.row_sessions
            .iter()
            .enumerate()
            .filter_map(move |(position, session_index)| {
                let session = (*session_index).and_then(|idx| self.sessions.get(idx))?;
                predicate(session).then_some(position)
            })
    }

    pub(super) fn selection_anchor(&self) -> SelectionAnchor {
        SelectionAnchor {
            position: self.list_state.selected(),
            session_id: self
                .selected_session()
                .map(|session| session.session_id.clone()),
            occurrence: self.selected_session_row_occurrence().unwrap_or_default(),
        }
    }

    /// Returns the selected session's occurrence among its rows in display order.
    /// A session with a YOUR TURN row also appears later in TASKS.
    pub(super) fn selected_session_row_occurrence(&self) -> Option<usize> {
        let position = self.list_state.selected()?;
        let session_index = self.row_sessions.get(position).copied().flatten()?;
        let preceding_rows = self.row_sessions.get(..position)?;
        Some(
            preceding_rows
                .iter()
                .filter(|row| **row == Some(session_index))
                .count(),
        )
    }

    pub(super) fn position_of_session_row_where(
        &self,
        predicate: impl Fn(&Session) -> bool,
    ) -> Option<usize> {
        self.session_row_positions_where(predicate).next()
    }

    /// Resolves the selection after a row-order rebuild.
    ///
    /// Selection follows a session by id across status changes / section
    /// moves whenever that session is still displayed. Otherwise (the
    /// session was filtered out entirely) falls back to the first
    /// selectable position at or after the previous one, else the last
    /// selectable position, else `None`.
    pub(super) fn resync_selection(&mut self, anchor: SelectionAnchor) {
        if let Some(id) = anchor.session_id
            && let Some(pos) = self.position_of_session_row_occurrence(&id, anchor.occurrence)
        {
            self.list_state.select(Some(pos));
            return;
        }

        let selectable = self.selectable_positions();
        let next = anchor
            .position
            .and_then(|p| {
                selectable
                    .iter()
                    .find(|&&i| i >= p)
                    .copied()
                    .or_else(|| selectable.last().copied())
            })
            .or_else(|| selectable.first().copied());
        self.list_state.select(next);
    }

    /// Restores a captured selection after rebuilding the row order.
    ///
    /// Rebuilds the row order from the current filtered sessions so that
    /// the cursor position is resolved against the actual display order,
    /// not the flat `updated_at` sort order.
    ///
    /// Capture the anchor before any mutation that can reset `list_state`.
    pub(super) fn restore_selection(&mut self, anchor: SelectionAnchor) {
        self.rebuild_row_order();
        self.resync_selection(anchor);
    }

    /// Persists the currently selected session ID to disk.
    /// Ignores errors to avoid disrupting UX.
    pub(super) fn persist_selection(&self) {
        if let Some(session) = self.selected_session() {
            let _ = store::save_last_selected_session(&session.session_id);
        }
    }

    /// Rebuilds the title cache for all sessions.
    pub(super) fn rebuild_title_cache(&mut self) {
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
/// Ended and Paused sessions are never considered stale -- they are retained
/// for `claude -c` resume even after their pane dies.
fn is_session_stale(session: &Session) -> bool {
    if !tmux::is_server_available() {
        return false;
    }
    if matches!(session.status, SessionStatus::Ended | SessionStatus::Paused) {
        return false;
    }
    session
        .tmux_info
        .as_ref()
        .is_some_and(|info| !tmux::is_pane_alive(&info.pane_id))
}

/// Builds the title cache for all sessions.
pub(super) fn build_title_cache(sessions: &[Session]) -> HashMap<String, String> {
    sessions
        .iter()
        .map(|session| (session.session_id.clone(), get_title_display_name(session)))
        .collect()
}

/// Gets the title display name for a session.
/// Priority: label (armyknife) > last custom-title entry > last ai-title
/// entry > .jsonl first user prompt > cwd basename.
/// All outputs are sanitized to strip ANSI escape sequences.
pub(super) fn get_title_display_name(session: &Session) -> String {
    // Prefer armyknife's own label (set via env var or auto-generated)
    if let Some(ref label) = session.label {
        return claude_sessions::normalize_title(label);
    }

    if let Some(title) = claude_sessions::get_session_title(&session.cwd, &session.session_id) {
        return title;
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

/// Loads sessions from disk.
///
/// Does not perform stale-session cleanup; that runs once at startup in
/// a background thread (see `EventHandler::new`).
pub(super) fn load_sessions() -> Result<Vec<Session>> {
    session_status::list_sessions_with_bg_run_status()
}

/// Lists explicitly enabled vibe sessions, including ended ancestors that
/// remain stored and continue to affect descendant sessions.
pub(super) fn load_vibe_enabled_session_ids() -> Result<HashSet<String>> {
    Ok(store::list_all_sessions()?
        .into_iter()
        .filter(|session| session.vibe_enabled)
        .map(|session| session.session_id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::tui::session_rows::SessionTask;
    use crate::commands::agent::tui::tq_snapshot::TqSnapshot;
    use chrono::{TimeDelta, Utc};
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
            pr_context: None,
            session_id: id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            is_delegated: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
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
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Engine::Claude,
        }
    }

    fn create_test_app(sessions: Vec<Session>) -> App {
        App::with_sessions(sessions)
    }

    #[test]
    fn test_modified_ancestor_disabling_vibe_clears_inherited_task_marker() {
        let cache_home = tempfile::tempdir().expect("temp cache home should be created");
        let cache_home_path = cache_home.path().to_string_lossy().into_owned();
        let sessions_dir = cache_home
            .path()
            .join("armyknife")
            .join("cc")
            .join("sessions");
        let mut ancestor = create_test_session("ancestor-session");
        ancestor.status = SessionStatus::Ended;
        store::save_session_to(&sessions_dir, &ancestor).expect("ancestor session should save");

        let mut descendant = create_test_session("descendant-session");
        descendant
            .ancestor_session_ids
            .push(ancestor.session_id.clone());
        let mut app = create_test_app(vec![descendant]);
        app.vibe_enabled_session_ids
            .insert(ancestor.session_id.clone());

        temp_env::with_vars([("XDG_CACHE_HOME", Some(cache_home_path.as_str()))], || {
            app.reload_sessions(Some(&[SessionChange {
                session_id: ancestor.session_id,
                change_type: SessionChangeType::Modified,
            }]))
            .expect("incremental reload should succeed");
        });

        let sessions = app.sessions.iter().collect::<Vec<_>>();
        let rows = build_session_rows_with_vibe(
            &sessions,
            &app.task_by_session,
            &app.agent_config,
            &app.vibe_enabled_session_ids,
        );
        let vibe_modes = rows
            .into_iter()
            .filter_map(|row| match row {
                super::super::super::session_rows::SessionRow::Session(entry) => {
                    Some(entry.vibe_mode)
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(vibe_modes, vec![None]);
    }

    // =========================================================================
    // Row-order / selection-persistence tests (TRIAGE inbox layout)
    // =========================================================================

    #[fixture]
    fn app_with_user_turn_and_active_session() -> App {
        let mut waiting = create_test_session("awaiting-1");
        waiting.status = SessionStatus::WaitingInput;
        waiting
            .crit_urls
            .push("https://example.com/review".to_string());
        create_test_app(vec![waiting, create_test_session("active-1")])
    }

    #[test]
    fn test_selection_follows_session_across_status_change() {
        let running_1 = create_test_session("running-1");
        let mut running_2 = create_test_session("running-2");
        running_2
            .crit_urls
            .push("https://example.com/review".to_string());
        let mut app = create_test_app(vec![running_1, running_2]);
        app.select_by_number(2);
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("running-2")
        );

        // Move the selected session to a different (still individually
        // selectable) section, then re-resolve the row order/selection the
        // same way `reload_sessions` does internally after a mutation.
        let selection = app.selection_anchor();
        app.sessions
            .iter_mut()
            .find(|s| s.session_id == "running-2")
            .expect("running-2 exists")
            .status = SessionStatus::WaitingInput;
        app.restore_selection(selection);

        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("running-2")
        );
    }

    #[rstest]
    fn test_reload_restores_selected_tasks_occurrence_for_user_turn_session(
        app_with_user_turn_and_active_session: App,
    ) {
        let mut app = app_with_user_turn_and_active_session;

        let task_position = app
            .position_of_session_row_occurrence("awaiting-1", 1)
            .expect("TASKS row is displayed");
        app.list_state.select(Some(task_position));
        app.apply_incremental_changes(&[SessionChange {
            session_id: "active-1".to_string(),
            change_type: SessionChangeType::Deleted,
        }])
        .expect("incremental reload succeeds");

        assert_eq!(
            (
                app.selected_session().map(|s| s.session_id.as_str()),
                app.selected_session_row_occurrence(),
            ),
            (Some("awaiting-1"), Some(1)),
        );
    }

    #[rstest]
    fn test_tq_snapshot_updates_preserve_selected_task_occurrence(
        app_with_user_turn_and_active_session: App,
    ) {
        let mut app = app_with_user_turn_and_active_session;

        let initial_task = SessionTask {
            task_id: "initial-task".to_string(),
            task_number: 10,
            task_title: "Initial title".to_string(),
            parent_task_id: None,
            is_closed: false,
        };
        app.set_tq_snapshot(TqSnapshot::new(HashMap::from([(
            "awaiting-1".to_string(),
            vec![initial_task.clone()],
        )])));
        app.rebuild_row_order();

        assert_eq!(
            (
                app.selected_session()
                    .map(|session| session.session_id.as_str()),
                app.selected_session_row_occurrence(),
                app.task_by_session.clone(),
            ),
            (
                Some("awaiting-1"),
                Some(0),
                HashMap::from([("awaiting-1".to_string(), initial_task)]),
            ),
        );

        let task_position = app
            .position_of_session_row_occurrence("awaiting-1", 1)
            .expect("TASKS row is displayed");
        app.list_state.select(Some(task_position));

        let refreshed_task = SessionTask {
            task_id: "refreshed-task".to_string(),
            task_number: 20,
            task_title: "Refreshed title".to_string(),
            parent_task_id: None,
            is_closed: false,
        };
        app.set_tq_snapshot(TqSnapshot::new(HashMap::from([(
            "awaiting-1".to_string(),
            vec![refreshed_task.clone()],
        )])));
        app.rebuild_row_order();

        assert_eq!(
            (
                app.selected_session()
                    .map(|session| session.session_id.as_str()),
                app.selected_session_row_occurrence(),
                app.task_by_session.clone(),
            ),
            (
                Some("awaiting-1"),
                Some(1),
                HashMap::from([("awaiting-1".to_string(), refreshed_task)]),
            ),
        );
    }

    #[test]
    fn test_restore_selection_fallback_anchors_on_pre_reload_position() {
        let mut app = create_test_app(vec![
            create_test_session("a"),
            create_test_session("b"),
            create_test_session("c"),
        ]);
        app.select_by_number(3);
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("c")
        );
        let selection = app.selection_anchor();

        // "c" is about to be excluded by a status filter -- mimics a status
        // change observed mid-reload that moves the selected session out of
        // the filtered set entirely.
        app.sessions
            .iter_mut()
            .find(|s| s.session_id == "c")
            .expect("c exists")
            .status = SessionStatus::Paused;
        app.toggle_status_filter(SessionStatus::Running);
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("a")
        );

        app.restore_selection(selection);

        // "c" no longer matches the filter and has no row at all; falls
        // back to the nearest selectable row at/after its own old position
        // (3) -- "b", the last selectable row -- not "a".
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("b")
        );
    }

    #[test]
    fn test_selection_falls_back_when_session_filtered_out() {
        let mut session_a = create_test_session("a");
        session_a.cwd = PathBuf::from("/home/user/keep1");
        let mut session_b = create_test_session("b");
        session_b.cwd = PathBuf::from("/home/user/drop");
        let mut session_c = create_test_session("c");
        session_c.cwd = PathBuf::from("/home/user/keep2");

        let mut app = create_test_app(vec![session_a, session_b, session_c]);
        app.select_by_number(2);
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("b")
        );

        // Set the query directly (skipping `update_search_query`'s live
        // per-keystroke filtering) so `confirm_search` resolves against
        // "b" -- the selection made against the *unfiltered* list -- not
        // against whatever the live filter would have already picked.
        app.enter_search_mode();
        app.search_query = "keep".to_string();
        app.confirm_search();

        // "b" no longer matches and has no row at all; falls back to the
        // first selectable row at or after its old position ("c").
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("c")
        );
    }
}
