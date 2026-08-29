use crate::commands::cc::claude_sessions;
use crate::commands::cc::store;
use crate::commands::cc::types::{Session, SessionStatus};
use crate::infra::tmux;
use anyhow::Result;
use std::collections::HashMap;

use super::super::event::{SessionChange, SessionChangeType};
use super::super::session_rows::build_session_rows;
use super::App;

impl App {
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
        // Remember the currently selected session_id and row position
        // before any mutation below can reset `list_state` (`apply_filter`
        // always does).
        let selected_session_id = self.selected_session().map(|s| s.session_id.clone());
        let old_pos = self.list_state.selected();

        self.sessions = load_sessions()?;

        // Rebuild title cache for new/changed sessions
        self.rebuild_title_cache();

        // Incrementally update searchable text cache if it exists
        if self.searchable_text_cache.is_some() {
            self.update_searchable_text_cache();
        }

        // Re-apply filter with current query
        self.apply_filter();
        self.restore_selection(old_pos, selected_session_id.as_deref());

        Ok(())
    }

    /// Applies incremental changes to the session list.
    fn apply_incremental_changes(&mut self, changes: &[SessionChange]) -> Result<()> {
        // Same ordering requirement as `full_reload`: capture the position
        // before `apply_filter` below can reset it.
        let selected_session_id = self.selected_session().map(|s| s.session_id.clone());
        let old_pos = self.list_state.selected();

        for change in changes {
            match change.change_type {
                SessionChangeType::Created | SessionChangeType::Modified => {
                    // Load the specific session
                    if let Some(session) = store::load_session(&change.session_id)? {
                        // Check if session is stale (TTY check)
                        if is_session_stale(&session) {
                            self.remove_session(&change.session_id);
                            store::delete_session(&change.session_id)?;
                        } else if session.status == SessionStatus::Ended {
                            // Ended sessions are not displayed; remove from list
                            self.remove_session(&change.session_id);
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

        // Re-sort with stability threshold to prevent rapid reordering
        store::sort_sessions(&mut self.sessions);

        // Rebuild caches for changed sessions only
        self.rebuild_title_cache_incremental(changes);

        if self.searchable_text_cache.is_some() {
            self.update_searchable_text_cache();
        }

        self.apply_filter();
        self.restore_selection(old_pos, selected_session_id.as_deref());

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
        let rows = build_session_rows(&filtered, &self.task_by_session);
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
        self.row_sessions.iter().position(|opt| {
            opt.and_then(|idx| self.sessions.get(idx))
                .is_some_and(|s| s.session_id == session_id)
        })
    }

    /// Resolves the selection after a row-order rebuild.
    ///
    /// Selection follows a session by id across status changes / section
    /// moves whenever that session is still displayed. Otherwise (the
    /// session was filtered out entirely) falls back to the first
    /// selectable position at or after the previous one, else the last
    /// selectable position, else `None`.
    pub(super) fn resync_selection(
        &mut self,
        old_pos: Option<usize>,
        old_session_id: Option<String>,
    ) {
        if let Some(id) = old_session_id
            && let Some(pos) = self.position_of_session_row(&id)
        {
            self.list_state.select(Some(pos));
            return;
        }

        let selectable = self.selectable_positions();
        let next = old_pos
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

    /// Restores selection by session_id if possible, otherwise adjusts.
    ///
    /// Rebuilds the row order from the current filtered sessions so that
    /// the cursor position is resolved against the actual display order,
    /// not the flat `updated_at` sort order.
    ///
    /// `old_pos` must be captured by the caller *before* any prior mutation
    /// (e.g. `apply_filter`) that could have already reset `list_state` --
    /// otherwise the fallback in [`Self::resync_selection`] anchors on that
    /// reset position instead of the user's actual previous cursor.
    pub(super) fn restore_selection(&mut self, old_pos: Option<usize>, session_id: Option<&str>) {
        self.rebuild_row_order();
        self.resync_selection(old_pos, session_id.map(String::from));
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
    store::list_sessions()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeDelta, Utc};
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
    // Row-order / selection-persistence tests (TRIAGE inbox layout)
    // =========================================================================

    #[test]
    fn test_selection_follows_session_across_status_change() {
        let mut app = create_test_app(vec![
            create_test_session("running-1"),
            create_test_session("running-2"),
        ]);
        app.select_by_number(2);
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("running-2")
        );

        // Move the selected session to a different (still individually
        // selectable) section, then re-resolve the row order/selection the
        // same way `reload_sessions` does internally after a mutation.
        let old_pos = app.list_state.selected();
        app.sessions
            .iter_mut()
            .find(|s| s.session_id == "running-2")
            .expect("running-2 exists")
            .status = SessionStatus::WaitingInput;
        app.restore_selection(old_pos, Some("running-2"));

        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("running-2")
        );
    }

    #[test]
    fn test_restore_selection_fallback_anchors_on_pre_reload_position() {
        // Regression test: `restore_selection`'s `old_pos` must be the
        // cursor position from *before* the caller's own `apply_filter`
        // call (as `full_reload`/`apply_incremental_changes` do), not
        // whatever `apply_filter` already reset `list_state` to -- otherwise
        // the fallback always lands on the first selectable row instead of
        // the nearest one to where the user actually was.
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
        let old_pos = app.list_state.selected();

        // "c" is about to be excluded by a status filter -- mimics a status
        // change observed mid-reload that moves the selected session out of
        // the filtered set entirely.
        app.sessions
            .iter_mut()
            .find(|s| s.session_id == "c")
            .expect("c exists")
            .status = SessionStatus::Paused;
        // Applying the filter resets `list_state` to the first selectable
        // row ("a") before `restore_selection` runs, same as every real
        // caller's `apply_filter(); restore_selection(...);` sequence.
        app.toggle_status_filter(SessionStatus::Running);
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("a")
        );

        app.restore_selection(old_pos, Some("c"));

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
