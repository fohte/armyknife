use crate::commands::agent::claude_sessions;
use crate::commands::agent::session_status;
use crate::commands::agent::store;
#[cfg(test)]
use crate::commands::agent::types::Engine;
use crate::commands::agent::types::{Session, SessionStatus};
use crate::infra::tmux;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::super::session_rows::{SessionRowOrder, build_session_rows_with_vibe};
use super::App;

const SESSION_ORDER_GRACE_PERIOD: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(super) struct SelectionAnchor {
    position: Option<usize>,
    session_id: Option<String>,
    screen_line: Option<usize>,
}

impl SelectionAnchor {
    pub(super) fn for_session(session_id: impl Into<String>) -> Self {
        Self {
            position: None,
            session_id: Some(session_id.into()),
            screen_line: None,
        }
    }
}

impl App {
    /// Applies every session-store revision newer than the current watermark.
    pub(crate) fn reload_session_store_changes(&mut self) -> Result<(bool, bool)> {
        let sessions_dir = store::sessions_dir()?;
        let mut changes = store::session_changes_since(&sessions_dir, self.session_store_revision)?;
        for session in &mut changes.changed_sessions {
            session_status::include_pending_status(session);
        }
        self.apply_session_store_changes(changes)
    }

    pub(crate) fn apply_session_store_changes(
        &mut self,
        changes: store::SessionStoreChanges,
    ) -> Result<(bool, bool)> {
        let selection = self.selection_anchor();
        let revision_changed = changes.revision > self.session_store_revision;
        let mut visible_state_changed = false;
        let mut new_session_created = false;
        let mut changed_ids = HashSet::new();

        for session in changes.changed_sessions {
            let session_id = session.session_id.clone();
            let was_visible = self
                .sessions
                .iter()
                .any(|existing| existing.session_id == session_id);
            changed_ids.insert(session_id.clone());

            if session.archived_at.is_some() {
                self.remove_session(&session_id);
                self.vibe_enabled_session_ids.remove(&session_id);
                visible_state_changed |= was_visible;
            } else {
                if session.vibe_enabled {
                    self.vibe_enabled_session_ids.insert(session_id.clone());
                } else {
                    self.vibe_enabled_session_ids.remove(&session_id);
                }

                if is_session_stale(&session) {
                    self.remove_session(&session_id);
                    store::delete_session(&session_id)?;
                    self.vibe_enabled_session_ids.remove(&session_id);
                    visible_state_changed |= was_visible;
                } else if session.status == SessionStatus::Ended {
                    self.remove_session(&session_id);
                    visible_state_changed |= was_visible;
                } else {
                    self.upsert_session(session);
                    visible_state_changed = true;
                    new_session_created |= !was_visible;
                }
            }
        }

        let removed_ids = self
            .sessions
            .iter()
            .filter(|session| !changes.existing_session_ids.contains(&session.session_id))
            .map(|session| session.session_id.clone())
            .collect::<Vec<_>>();
        for session_id in removed_ids {
            self.remove_session(&session_id);
            self.vibe_enabled_session_ids.remove(&session_id);
            changed_ids.insert(session_id);
            visible_state_changed = true;
        }

        let removed_vibe_ids = self
            .vibe_enabled_session_ids
            .difference(&changes.existing_session_ids)
            .cloned()
            .collect::<Vec<_>>();
        for session_id in removed_vibe_ids {
            self.vibe_enabled_session_ids.remove(&session_id);
            changed_ids.insert(session_id);
            visible_state_changed = true;
        }

        if let Some(order) = self.session_row_order.as_mut() {
            order.retain_sessions(&changes.existing_session_ids);
        }

        for session_id in &changed_ids {
            if let Some(session) = self
                .sessions
                .iter()
                .find(|session| session.session_id == *session_id)
            {
                self.title_cache
                    .insert(session_id.clone(), get_title_display_name(session));
            } else {
                self.title_cache.remove(session_id);
            }
        }

        if visible_state_changed {
            store::sort_sessions(&mut self.sessions);

            if self.searchable_text_cache.is_some() {
                self.update_searchable_text_cache();
            }

            self.apply_filter();
            self.restore_selection(selection);
        }

        self.session_store_revision = changes.revision;
        Ok((revision_changed, new_session_created))
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
        let rows = self.build_display_rows();
        let row_keys = rows
            .iter()
            .map(|row| row.session_id().map(str::to_owned))
            .collect::<Vec<_>>();
        drop(rows);
        self.update_rendered_row_keys(&row_keys);
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
    pub(crate) fn position_of_session_row(&self, session_id: &str) -> Option<usize> {
        self.position_of_session_row_where(|session| session.session_id == session_id)
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
        let position = self.list_state.selected();
        SelectionAnchor {
            position,
            session_id: position
                .and_then(|position| self.row_keys.get(position).cloned().flatten())
                .or_else(|| {
                    self.selected_session()
                        .map(|session| session.session_id.clone())
                }),
            screen_line: position.and_then(|position| {
                self.pending_selection_line
                    .or_else(|| self.selection_screen_line(position))
            }),
        }
    }

    fn selection_screen_line(&self, selected: usize) -> Option<usize> {
        let offset = self.list_state.offset().min(selected);
        self.row_heights
            .get(offset..selected)
            .map(|heights| heights.iter().sum())
    }

    pub(super) fn note_session_list_interaction(&mut self) {
        self.note_session_list_interaction_at(Instant::now());
    }

    pub(crate) fn note_session_list_interaction_at(&mut self, now: Instant) {
        if self.session_row_order.is_none() {
            let all_sessions = self.sessions.iter().collect::<Vec<_>>();
            let rows = build_session_rows_with_vibe(
                &all_sessions,
                &self.task_by_session,
                &self.agent_config,
                &self.vibe_enabled_session_ids,
            );
            self.session_row_order = Some(SessionRowOrder::from_rows(&rows));
        }
        self.navigation_hold_deadline = Some(now + SESSION_ORDER_GRACE_PERIOD);
    }

    pub(crate) fn navigation_hold_deadline(&self) -> Option<Instant> {
        self.navigation_hold_deadline
    }

    pub(crate) fn finish_navigation_hold_if_due(&mut self, now: Instant) -> bool {
        if !self
            .navigation_hold_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            return false;
        }

        let selection = self.selection_anchor();
        self.navigation_hold_deadline = None;
        self.session_row_order = None;
        self.restore_selection(selection);
        true
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
        let next = anchor
            .session_id
            .as_ref()
            .and_then(|session_id| self.position_of_session_row(session_id))
            .or_else(|| {
                let selectable = self.selectable_positions();
                anchor.position.and_then(|position| {
                    selectable
                        .iter()
                        .find(|&&index| index >= position)
                        .copied()
                        .or_else(|| selectable.last().copied())
                })
            })
            .or_else(|| self.selectable_positions().first().copied());
        self.list_state.select(next);
        self.pending_selection_line = next.and(anchor.screen_line);
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
}

/// Checks if a session is stale (TTY no longer exists).
/// Ended and Paused sessions are never considered stale -- they are retained
/// for `claude -c` resume even after their pane dies.
fn is_session_stale(session: &Session) -> bool {
    if matches!(session.status, SessionStatus::Ended | SessionStatus::Paused) {
        return false;
    }
    let Some(info) = session.tmux_info.as_ref() else {
        return false;
    };
    tmux::is_pane_alive_for_socket(info.socket_path.as_deref(), &info.pane_id)
        .is_ok_and(|is_alive| !is_alive)
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
    use chrono::{TimeDelta, TimeZone, Utc};
    use rstest::{fixture, rstest};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    /// Counter to assign distinct timestamps to test sessions.
    /// Each call returns a progressively older timestamp, so sessions
    /// created first sort first (most recent updated_at).
    use std::sync::atomic::{AtomicI64, Ordering};
    static TEST_SESSION_COUNTER: AtomicI64 = AtomicI64::new(0);

    fn create_test_session(id: &str) -> Session {
        let offset = TEST_SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let now = Utc::now();
        Session {
            archived_at: None,
            pr_context: None,
            session_id: id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
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
            model: None,
            effort: None,
        }
    }

    fn create_test_app(sessions: Vec<Session>) -> App {
        App::with_sessions(sessions)
    }

    #[test]
    fn test_session_store_changes_hide_archived_ended_and_deleted_sessions() {
        let mut deleted = create_test_session("deleted-session");
        deleted.vibe_enabled = true;
        let archived = {
            let mut session = create_test_session("archived-session");
            session.archived_at = Some(Utc::now());
            session.vibe_enabled = true;
            session
        };
        let ended = {
            let mut session = create_test_session("ended-session");
            session.status = SessionStatus::Ended;
            session.vibe_enabled = true;
            session
        };
        let mut app = create_test_app(vec![deleted, create_test_session("archived-session"), {
            let mut session = create_test_session("ended-session");
            session.vibe_enabled = true;
            session
        }]);

        app.apply_session_store_changes(store::SessionStoreChanges {
            changed_sessions: vec![archived, ended],
            existing_session_ids: HashSet::from([
                "archived-session".to_string(),
                "ended-session".to_string(),
            ]),
            revision: 4,
        })
        .expect("session-store changes should apply");

        assert_eq!(
            (
                app.sessions
                    .iter()
                    .map(|session| session.session_id.as_str())
                    .collect::<Vec<_>>(),
                app.vibe_enabled_session_ids.clone(),
                app.session_store_revision,
            ),
            (
                Vec::<&str>::new(),
                HashSet::from(["ended-session".to_string()]),
                4,
            ),
        );
    }

    #[test]
    fn test_session_store_revision_delta_updates_and_removes_sessions() {
        let temp_dir = tempfile::tempdir().expect("temp directory should be created");
        let sessions_dir = temp_dir.path().join("sessions");
        let mut kept = create_test_session("retained-row");
        kept.label = Some("Before update".to_string());
        let mut deleted = create_test_session("removed-row");
        deleted.vibe_enabled = true;
        store::save_session_to(&sessions_dir, &kept).expect("session should save");
        store::save_session_to(&sessions_dir, &deleted).expect("session should save");
        let initial_revision = store::session_store_revision_from(&sessions_dir)
            .expect("initial revision should load");
        let mut app = create_test_app(vec![kept, deleted]);

        store::update_session_label_in(
            &sessions_dir,
            "retained-row",
            Some("After update".to_string()),
        )
        .expect("session should update");
        store::delete_session_from(&sessions_dir, "removed-row").expect("session should delete");
        let changes = store::session_changes_since(&sessions_dir, initial_revision)
            .expect("session changes should load");
        let revision = changes.revision;

        app.apply_session_store_changes(changes)
            .expect("session changes should apply");

        assert_eq!(
            (
                app.sessions
                    .iter()
                    .map(|session| (session.session_id.clone(), session.label.clone()))
                    .collect::<Vec<_>>(),
                app.vibe_enabled_session_ids.clone(),
                app.session_store_revision,
            ),
            (
                vec![("retained-row".to_string(), Some("After update".to_string()))],
                HashSet::new(),
                revision,
            ),
        );
    }

    #[test]
    fn test_modified_ancestor_disabling_vibe_clears_inherited_task_marker() {
        let temp_dir = tempfile::tempdir().expect("temp directory should be created");
        let sessions_dir = temp_dir.path().join("sessions");
        let mut ancestor = create_test_session("ancestor-session");
        ancestor.status = SessionStatus::Ended;
        ancestor.vibe_enabled = true;
        store::save_session_to(&sessions_dir, &ancestor).expect("ancestor session should save");
        let initial_revision = store::session_store_revision_from(&sessions_dir)
            .expect("initial revision should load");

        let mut descendant = create_test_session("descendant-session");
        descendant
            .ancestor_session_ids
            .push(ancestor.session_id.clone());
        store::save_session_to(&sessions_dir, &descendant).expect("descendant session should save");
        let mut app = create_test_app(vec![descendant]);
        app.vibe_enabled_session_ids
            .insert(ancestor.session_id.clone());
        app.session_store_revision = initial_revision;

        store::update_session_in(&sessions_dir, &ancestor.session_id, |ancestor| {
            ancestor.vibe_enabled = false;
            true
        })
        .expect("ancestor should update");
        let changes = store::session_changes_since(&sessions_dir, initial_revision)
            .expect("session changes should load");
        app.apply_session_store_changes(changes)
            .expect("session changes should apply");

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
        // selectable) section, then re-resolve the row order and selection.
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
    fn test_reload_restores_selected_task_row_for_user_turn_session(
        app_with_user_turn_and_active_session: App,
    ) {
        let mut app = app_with_user_turn_and_active_session;

        let task_position = app
            .position_of_session_row("awaiting-1")
            .expect("TASKS row is displayed");
        app.list_state.select(Some(task_position));
        app.apply_session_store_changes(store::SessionStoreChanges {
            changed_sessions: Vec::new(),
            existing_session_ids: HashSet::from(["awaiting-1".to_string()]),
            revision: 1,
        })
        .expect("session-store changes should apply");

        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("awaiting-1"),
        );
    }

    #[test]
    fn navigation_holds_existing_order_until_two_seconds_after_the_last_interaction() {
        let t0 = Utc
            .timestamp_opt(1_000, 0)
            .single()
            .expect("valid timestamp");
        let t1 = t0 + TimeDelta::seconds(1);
        let t2 = t0 + TimeDelta::seconds(2);
        let t3 = t0 + TimeDelta::seconds(3);
        let t4 = t0 + TimeDelta::seconds(4);
        let mut root_a = create_test_session("root-a");
        root_a.updated_at = t0;
        let mut child_a = create_test_session("child-a");
        child_a.updated_at = t2;
        child_a.ancestor_session_ids = vec!["root-a".to_string()];
        let mut child_b = create_test_session("child-b");
        child_b.updated_at = t1;
        child_b.ancestor_session_ids = vec!["root-a".to_string()];
        let mut root_b = create_test_session("root-b");
        root_b.updated_at = t3;
        let mut app = create_test_app(vec![root_a, child_a, child_b, root_b]);
        let selected_position = app
            .position_of_session_row("child-a")
            .expect("selected row is displayed");
        app.list_state.select(Some(selected_position));
        let start = Instant::now();
        app.note_session_list_interaction_at(start);

        let mut updated_child_b = app
            .sessions
            .iter()
            .find(|session| session.session_id == "child-b")
            .expect("child-b exists")
            .clone();
        updated_child_b.updated_at = t4;
        app.apply_session_store_changes(store::SessionStoreChanges {
            changed_sessions: vec![updated_child_b],
            existing_session_ids: HashSet::from([
                "root-a".to_string(),
                "child-a".to_string(),
                "child-b".to_string(),
                "root-b".to_string(),
            ]),
            revision: 1,
        })
        .expect("session-store changes should apply");
        app.note_session_list_interaction_at(start + Duration::from_secs(1));

        let held_order_and_state = (
            app.row_keys.clone(),
            app.selected_session()
                .map(|session| session.session_id.clone()),
            app.sessions
                .iter()
                .find(|session| session.session_id == "child-b")
                .map(|session| session.updated_at),
            app.navigation_hold_deadline(),
        );
        let early = app.finish_navigation_hold_if_due(start + Duration::from_secs(2));
        let almost = app.finish_navigation_hold_if_due(start + Duration::from_millis(2_999));
        let applied = app.finish_navigation_hold_if_due(start + Duration::from_secs(3));

        assert_eq!(
            (
                held_order_and_state,
                early,
                almost,
                applied,
                app.row_keys.clone(),
                app.selected_session()
                    .map(|session| session.session_id.clone()),
            ),
            (
                (
                    vec![
                        None,
                        Some("root-b".to_string()),
                        Some("root-a".to_string()),
                        Some("child-a".to_string()),
                        Some("child-b".to_string()),
                    ],
                    Some("child-a".to_string()),
                    Some(t4),
                    Some(start + Duration::from_secs(3)),
                ),
                false,
                false,
                true,
                vec![
                    None,
                    Some("root-a".to_string()),
                    Some("child-b".to_string()),
                    Some("child-a".to_string()),
                    Some("root-b".to_string()),
                ],
                Some("child-a".to_string()),
            ),
        );
    }

    #[test]
    fn new_children_are_appended_after_existing_siblings_during_navigation() {
        let t0 = Utc
            .timestamp_opt(2_000, 0)
            .single()
            .expect("valid timestamp");
        let mut root = create_test_session("root");
        root.updated_at = t0;
        let mut existing_child = create_test_session("existing-child");
        existing_child.updated_at = t0 + TimeDelta::seconds(1);
        existing_child.ancestor_session_ids = vec!["root".to_string()];
        let mut app = create_test_app(vec![root, existing_child]);
        app.note_session_list_interaction_at(Instant::now());

        let mut new_child = create_test_session("new-child");
        new_child.updated_at = t0 + TimeDelta::seconds(2);
        new_child.ancestor_session_ids = vec!["root".to_string()];
        app.apply_session_store_changes(store::SessionStoreChanges {
            changed_sessions: vec![new_child],
            existing_session_ids: HashSet::from([
                "root".to_string(),
                "existing-child".to_string(),
                "new-child".to_string(),
            ]),
            revision: 1,
        })
        .expect("session-store changes should apply");

        assert_eq!(
            app.row_keys,
            vec![
                None,
                Some("root".to_string()),
                Some("existing-child".to_string()),
                Some("new-child".to_string()),
            ],
        );
    }

    #[test]
    fn task_row_remains_selected_when_session_needs_user_action() {
        let mut selected = create_test_session("selected");
        let other = create_test_session("other");
        let mut app = create_test_app(vec![selected.clone(), other]);
        let task_position = app
            .position_of_session_row("selected")
            .expect("TASKS row is displayed");
        app.list_state.select(Some(task_position));
        selected.status = SessionStatus::WaitingInput;
        selected
            .crit_urls
            .push("https://example.com/review".to_string());
        app.apply_session_store_changes(store::SessionStoreChanges {
            changed_sessions: vec![selected],
            existing_session_ids: HashSet::from(["selected".to_string(), "other".to_string()]),
            revision: 1,
        })
        .expect("session-store changes should apply");

        assert_eq!(
            (
                app.selected_session()
                    .map(|session| session.session_id.as_str()),
                app.row_keys
                    .get(app.list_state.selected().unwrap_or_default())
                    .cloned()
                    .flatten(),
            ),
            (Some("selected"), Some("selected".to_string()),),
        );
    }

    #[rstest]
    fn test_tq_snapshot_updates_preserve_selected_task(app_with_user_turn_and_active_session: App) {
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
                app.task_by_session.clone(),
            ),
            (
                Some("awaiting-1"),
                HashMap::from([("awaiting-1".to_string(), initial_task)]),
            ),
        );

        let task_position = app
            .position_of_session_row("awaiting-1")
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
                app.task_by_session.clone(),
            ),
            (
                Some("awaiting-1"),
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
            Some("b")
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
