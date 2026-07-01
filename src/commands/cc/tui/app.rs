use crate::commands::cc::claude_sessions;
use crate::commands::cc::store;
use crate::commands::cc::types::{Session, SessionStatus};
use crate::infra::tmux;
use anyhow::Result;
use chrono::{DateTime, Utc};
use ratatui::widgets::ListState;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::clean_progress::{CleanLogEvent, CleanProgress};
use super::clean_view::CleanView;
use super::event::{SessionChange, SessionChangeType};
use super::session_tree::build_session_tree;
use super::worktree_view::{
    WorktreeMode, WorktreeRow, WorktreeView, canonicalize_or_self, session_lives_under,
};

/// Top-level view selection. Tab cycles between Session and Worktree
/// only; `Clean` is reached via `c` and exited via Esc/n/q.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Session,
    Worktree,
    Clean,
}

impl View {
    pub fn next(self) -> Self {
        match self {
            View::Session => View::Worktree,
            View::Worktree => View::Session,
            View::Clean => View::Clean,
        }
    }
}

/// Application mode.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AppMode {
    #[default]
    Normal,
    Search,
    /// Confirm deletion of a session. Holds the session_id and its status.
    Confirm {
        session_id: String,
        is_alive: bool,
    },
    /// After deleting the last session in a worktree, ask whether to also
    /// remove the worktree itself (branch, tmux windows, worktree dir).
    /// `worktree_root` is the resolved worktree root path used for cleanup.
    ConfirmWorktreeCleanup {
        worktree_root: PathBuf,
    },
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
    /// Status filter: when set, only sessions with this status are shown.
    pub status_filter: Option<SessionStatus>,
    /// Cache of searchable text for each session (keyed by session_id).
    /// Lazily built when search mode is first entered.
    /// Stores (searchable_text, updated_at) for incremental updates.
    searchable_text_cache: Option<HashMap<String, (String, DateTime<Utc>)>>,
    /// Cache of session titles for display (keyed by session_id).
    /// Built on load/reload for fast UI rendering.
    title_cache: HashMap<String, String>,
    /// Cache of (repo_name, worktree_name) keyed by cwd path.
    /// Populated asynchronously; render must not block on libgit2 I/O.
    worktree_label_cache: HashMap<PathBuf, (String, String)>,
    /// Cwds whose async resolution is in flight. Guards `claim_unresolved_label_cwds`
    /// against re-dispatch before the corresponding result event arrives.
    pending_label_cwds: HashSet<PathBuf>,
    /// Tree-ordered indices into `sessions`.
    /// Updated each render by the UI layer after building the session tree.
    /// Maps display position (list_state index) to sessions index.
    tree_ordered_indices: Vec<usize>,
    /// Currently active top-level view.
    pub view: View,
    /// View to return to when the user exits the clean view (Esc/n/q).
    pub clean_return_view: View,
    /// Worktree-view state (background-loaded list, sub-mode, selection).
    pub worktree_view: WorktreeView,
    /// Clean-view state (sections, selection, PR-fetch progress).
    pub clean_view: CleanView,
    /// In-flight detached clean progress. `Some` from the moment the
    /// user confirms `y` in the clean view; cleared once the bottom-bar
    /// summary has been on screen long enough for the user to read it.
    pub clean_progress: Option<CleanProgress>,
}

impl App {
    /// Creates a new App instance with initial session data.
    /// Restores the last selected session if available.
    ///
    /// If `ARMYKNIFE_FOCUS_SESSION` is set, that session is selected instead of
    /// the persisted last-selected session. This allows tmux bindings to pass
    /// the currently focused pane's session ID via an environment variable.
    pub fn new() -> Result<Self> {
        let sessions = load_sessions()?;
        let mut app = Self::with_sessions(sessions);

        // Prefer ARMYKNIFE_FOCUS_SESSION over persisted selection
        let initial_session_id = std::env::var("ARMYKNIFE_FOCUS_SESSION")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| store::load_last_selected_session().ok().flatten());

        if let Some(session_id) = initial_session_id {
            app.restore_selection(Some(&session_id));
        }

        Ok(app)
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

        let mut app = Self {
            sessions,
            list_state,
            should_quit: false,
            error_message: None,
            mode: AppMode::Normal,
            search_query: String::new(),
            confirmed_query: String::new(),
            filtered_indices,
            pre_search_selection: None,
            status_filter: None,
            // Searchable text cache is lazily built on first search
            searchable_text_cache: None,
            tree_ordered_indices: Vec::new(),
            title_cache,
            worktree_label_cache: HashMap::new(),
            pending_label_cwds: HashSet::new(),
            view: View::Session,
            clean_return_view: View::Session,
            worktree_view: WorktreeView::new(),
            clean_view: CleanView::new(),
            clean_progress: None,
        };
        app.rebuild_tree_order();
        app
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

    /// Rebuilds `tree_ordered_indices` from the current `filtered_indices`.
    ///
    /// Runs the same DFS tree-building logic that the render layer uses,
    /// so that cursor positions always match the displayed order.
    fn rebuild_tree_order(&mut self) {
        let filtered: Vec<&Session> = self
            .filtered_indices
            .iter()
            .filter_map(|&i| self.sessions.get(i))
            .collect();
        let tree_entries = build_session_tree(&filtered);
        self.tree_ordered_indices = tree_entries
            .iter()
            .filter_map(|entry| {
                self.sessions
                    .iter()
                    .position(|s| s.session_id == entry.session.session_id)
            })
            .collect();
    }

    /// Restores selection by session_id if possible, otherwise adjusts.
    ///
    /// Rebuilds the tree order from the current filtered sessions so that
    /// the cursor position is resolved against the actual display order,
    /// not the flat `updated_at` sort order.
    fn restore_selection(&mut self, session_id: Option<&str>) {
        self.rebuild_tree_order();

        if let Some(id) = session_id
            && let Some(pos) = self
                .tree_ordered_indices
                .iter()
                .position(|&i| self.sessions.get(i).is_some_and(|s| s.session_id == id))
        {
            self.list_state.select(Some(pos));
            return;
        }

        // Fallback: adjust selection if needed
        if self.tree_ordered_indices.is_empty() {
            self.list_state.select(None);
        } else if let Some(selected) = self.list_state.selected() {
            if selected >= self.tree_ordered_indices.len() {
                self.list_state
                    .select(Some(self.tree_ordered_indices.len() - 1));
            }
        } else {
            self.list_state.select(Some(0));
        }
    }

    /// Persists the currently selected session ID to disk.
    /// Ignores errors to avoid disrupting UX.
    fn persist_selection(&self) {
        if let Some(session) = self.selected_session() {
            let _ = store::save_last_selected_session(&session.session_id);
        }
    }

    /// Moves selection to the next item in the displayed list.
    pub fn select_next(&mut self) {
        if self.tree_ordered_indices.is_empty() {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.tree_ordered_indices.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
        self.persist_selection();
    }

    /// Moves selection to the previous item in the displayed list.
    pub fn select_previous(&mut self) {
        if self.tree_ordered_indices.is_empty() {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.tree_ordered_indices.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
        self.persist_selection();
    }

    /// Selects a session by its 1-indexed number (1-9) within the displayed list.
    pub fn select_by_number(&mut self, num: usize) {
        if num > 0 && num <= self.tree_ordered_indices.len() {
            self.list_state.select(Some(num - 1));
            self.persist_selection();
        }
    }

    /// Returns the currently selected session, if any.
    /// Uses tree-ordered indices which reflect the actual display order
    /// after tree view reordering.
    pub fn selected_session(&self) -> Option<&Session> {
        self.list_state
            .selected()
            .and_then(|i| self.tree_ordered_indices.get(i))
            .and_then(|&session_idx| self.sessions.get(session_idx))
    }

    /// Returns the filtered sessions for display.
    pub fn filtered_sessions(&self) -> Vec<&Session> {
        self.filtered_indices
            .iter()
            .filter_map(|&i| self.sessions.get(i))
            .collect()
    }

    /// Updates tree-ordered indices from display-ordered session IDs.
    /// Called by the UI layer after building the session tree to keep
    /// the selection mapping in sync with the rendered list order.
    pub fn update_tree_order(&mut self, session_ids: &[&str]) {
        self.tree_ordered_indices = session_ids
            .iter()
            .filter_map(|id| self.sessions.iter().position(|s| s.session_id == *id))
            .collect();
    }

    /// Returns whether a filter is currently active.
    pub fn has_filter(&self) -> bool {
        !self.confirmed_query.is_empty() || self.status_filter.is_some()
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

    /// Cache lookup only. Misses are expected for sessions whose async
    /// resolution has not yet completed.
    pub fn get_cached_worktree_labels(&self, cwd: &std::path::Path) -> Option<(&str, &str)> {
        self.worktree_label_cache
            .get(cwd)
            .map(|(r, n)| (r.as_str(), n.as_str()))
    }

    /// Returns cwds present in `sessions` whose worktree labels are neither
    /// cached nor currently being resolved, and marks them as pending.
    /// Callers dispatch the returned list to a background resolver.
    pub fn claim_unresolved_label_cwds(&mut self) -> Vec<PathBuf> {
        let mut seen: HashSet<&Path> = HashSet::new();
        let mut out = Vec::new();
        for session in &self.sessions {
            let cwd = session.cwd.as_path();
            if !seen.insert(cwd) {
                continue;
            }
            if self.worktree_label_cache.contains_key(cwd) {
                continue;
            }
            if self.pending_label_cwds.contains(cwd) {
                continue;
            }
            out.push(cwd.to_path_buf());
        }
        for cwd in &out {
            self.pending_label_cwds.insert(cwd.clone());
        }
        out
    }

    /// Inserts the results of an async label resolution into the cache.
    pub fn apply_resolved_labels(&mut self, results: Vec<(PathBuf, String, String)>) {
        for (cwd, repo, worktree) in results {
            self.pending_label_cwds.remove(&cwd);
            self.worktree_label_cache.insert(cwd, (repo, worktree));
        }
    }

    /// Cycles the active view. No-op when in `Clean`.
    pub fn cycle_view(&mut self) {
        if self.view == View::Clean {
            return;
        }
        self.view = self.view.next();
        if self.view == View::Worktree {
            // Make sure overlay reflects the latest session list whenever the
            // user lands on the worktree view.
            self.worktree_view.refresh_session_overlay(&self.sessions);
        }
    }

    /// Installs the freshly loaded worktree rows.
    pub fn set_worktrees(&mut self, rows: Vec<WorktreeRow>) {
        self.worktree_view.set_rows(rows);
        self.worktree_view.refresh_session_overlay(&self.sessions);
    }

    /// Marks worktree discovery as failed (background thread error) and
    /// also surfaces the error in the global error banner so the user
    /// notices it without switching to the worktree view first.
    pub fn set_worktrees_failed(&mut self, error: String) {
        self.set_error(format!("Failed to load worktrees: {error}"));
        self.worktree_view.set_failed(error);
    }

    /// In worktree view, returns the most recently updated session inside the
    /// currently selected worktree (used for `Enter` → focus pane).
    pub fn worktree_view_focus_session(&self) -> Option<&Session> {
        let row = self.worktree_view.selected_worktree()?;
        // `row.path` is already canonicalized at discovery time.
        self.sessions
            .iter()
            .filter(|s| canonicalize_or_self(&s.cwd).starts_with(&row.path))
            .max_by_key(|s| s.updated_at)
    }

    /// Enters Confirm sub-mode on the selected worktree (for `d`).
    pub fn worktree_view_request_delete(&mut self) {
        if let Some(row) = self.worktree_view.selected_worktree() {
            self.worktree_view.mode = WorktreeMode::Confirm {
                worktree_path: row.path,
                session_count: row.session_count,
                has_active: row.has_active,
            };
        }
    }

    /// Cancels the pending worktree-view confirmation.
    pub fn worktree_view_cancel_confirm(&mut self) {
        self.worktree_view.mode = WorktreeMode::Normal;
    }

    /// Deletes the worktree via `cleanup_worktree_resources` (git worktree,
    /// branch, tmux windows, session files). Does not consult merge status.
    pub fn worktree_view_confirm_delete(&mut self) -> anyhow::Result<()> {
        let path = match &self.worktree_view.mode {
            WorktreeMode::Confirm { worktree_path, .. } => worktree_path.clone(),
            _ => return Ok(()),
        };

        self.worktree_view.mode = WorktreeMode::Normal;

        use crate::shared::cleanup;
        let result = cleanup::cleanup_worktree_resources(&path)?;

        if result.worktree_deleted {
            // Drop sessions whose cwd is gone.
            if let Some(ref wt_root) = result.worktree_root {
                let to_remove: Vec<String> = self
                    .sessions
                    .iter()
                    .filter(|s| session_lives_under(&s.cwd, wt_root))
                    .map(|s| s.session_id.clone())
                    .collect();
                for id in &to_remove {
                    self.remove_session(id);
                }
            }
            let prev_selection = self.worktree_view.list_state.selected();
            if let super::worktree_view::WorktreeLoadState::Loaded(rows) =
                &mut self.worktree_view.state
            {
                rows.retain(|r| r.path != path);
            }
            self.worktree_view.refresh_session_overlay(&self.sessions);
            // Keep the cursor near the deleted row: pick the first
            // selectable index >= the old position, otherwise the last.
            let sel = self.worktree_view.selectable_indices();
            let next = prev_selection
                .and_then(|p| {
                    sel.iter()
                        .find(|&&i| i >= p)
                        .copied()
                        .or_else(|| sel.last().copied())
                })
                .or_else(|| sel.first().copied());
            self.worktree_view.list_state.select(next);
        } else {
            self.set_error(format!(
                "Worktree not deleted: {} (use `a wm clean` to investigate)",
                path.display()
            ));
        }
        Ok(())
    }

    /// Exits search mode, confirming the search.
    /// Preserves the current selection position.
    pub fn confirm_search(&mut self) {
        let current_selection = self.list_state.selected();
        self.confirmed_query = self.search_query.clone();
        self.apply_filter();
        // Restore selection position (apply_filter resets to 0)
        if let Some(pos) = current_selection
            && pos < self.filtered_indices.len()
        {
            self.list_state.select(Some(pos));
        }
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

    /// Enters confirm-delete mode for the currently selected session.
    pub fn request_delete(&mut self) {
        if let Some(session) = self.selected_session() {
            let is_alive = session
                .tmux_info
                .as_ref()
                .is_some_and(|info| tmux::is_pane_alive(&info.pane_id));
            self.mode = AppMode::Confirm {
                session_id: session.session_id.clone(),
                is_alive,
            };
        }
    }

    /// Executes the confirmed delete action for a single session.
    /// If the session is alive, sends SIGTERM to the pane process first.
    ///
    /// After the session is removed, if it was the last session in its
    /// worktree, transitions to `ConfirmWorktreeCleanup` so the user can
    /// decide whether to also delete the worktree itself. Worktree cleanup
    /// is never performed silently: sibling sessions in the same worktree
    /// must not be touched by a single-session delete.
    pub fn confirm_delete(&mut self) -> anyhow::Result<()> {
        let current_selection = self.list_state.selected();
        let (session_id, is_alive) = match &self.mode {
            AppMode::Confirm {
                session_id,
                is_alive,
            } => (session_id.clone(), *is_alive),
            _ => return Ok(()),
        };

        // Capture cwd before removal so we can detect whether this session
        // was the last one in its worktree.
        let session_cwd = self
            .sessions
            .iter()
            .find(|s| s.session_id == session_id)
            .map(|s| s.cwd.clone());

        if is_alive
            && let Some(session) = self.sessions.iter().find(|s| s.session_id == session_id)
            && let Some(ref tmux_info) = session.tmux_info
        {
            tmux::send_sigterm_to_pane(&tmux_info.pane_id);
        }

        store::delete_session(&session_id)?;
        self.remove_session(&session_id);

        // Decide whether to prompt for worktree cleanup. Only prompt when the
        // deleted session was inside a git worktree AND no sibling sessions
        // remain in that worktree. Otherwise, leave the worktree intact.
        let next_mode = session_cwd
            .as_deref()
            .and_then(resolve_worktree_root)
            .filter(|wt_root| !self.has_session_in_worktree(wt_root))
            .map(|worktree_root| AppMode::ConfirmWorktreeCleanup { worktree_root });

        self.refresh_after_mutation(current_selection);
        self.mode = next_mode.unwrap_or(AppMode::Normal);
        Ok(())
    }

    /// Executes worktree cleanup after the user confirmed it from
    /// `ConfirmWorktreeCleanup` mode. Removes the worktree, its branch,
    /// associated tmux windows, and any remaining session files inside
    /// the worktree path (best-effort).
    pub fn confirm_worktree_cleanup(&mut self) -> anyhow::Result<()> {
        let current_selection = self.list_state.selected();
        let worktree_root = match &self.mode {
            AppMode::ConfirmWorktreeCleanup { worktree_root } => worktree_root.clone(),
            _ => return Ok(()),
        };

        // Always leave Confirm mode so the TUI is usable again, even if
        // cleanup fails. Errors propagate to the caller, which surfaces
        // them via `set_error`.
        self.mode = AppMode::Normal;

        use crate::shared::cleanup;
        let result = cleanup::cleanup_worktree_resources(&worktree_root)?;
        if let Some(ref wt_root) = result.worktree_root {
            // A race is possible: new sessions may have been created inside
            // the worktree between the first confirmation and this one.
            // cleanup_worktree_resources already removed their files, so
            // prune them from the in-memory list to stay consistent.
            let to_remove: Vec<String> = self
                .sessions
                .iter()
                .filter(|s| s.cwd.starts_with(wt_root))
                .map(|s| s.session_id.clone())
                .collect();
            for id in &to_remove {
                self.remove_session(id);
            }
        }

        self.refresh_after_mutation(current_selection);
        Ok(())
    }

    /// Cancels the confirm-delete or confirm-worktree-cleanup dialog.
    pub fn cancel_confirm(&mut self) {
        self.mode = AppMode::Normal;
    }

    /// Returns true if any session's cwd is inside `worktree_root`.
    fn has_session_in_worktree(&self, worktree_root: &Path) -> bool {
        self.sessions
            .iter()
            .any(|s| s.cwd.starts_with(worktree_root))
    }

    /// Re-sorts sessions, rebuilds caches, reapplies filters, and restores
    /// selection. Shared by `confirm_delete` and `confirm_worktree_cleanup`.
    fn refresh_after_mutation(&mut self, previous_selection: Option<usize>) {
        store::sort_sessions(&mut self.sessions);
        self.rebuild_title_cache();
        if self.searchable_text_cache.is_some() {
            self.update_searchable_text_cache();
        }
        self.apply_filter();

        if let Some(selected) = previous_selection {
            let new_len = self.tree_ordered_indices.len();
            if new_len > 0 {
                self.list_state.select(Some(selected.min(new_len - 1)));
            } else {
                self.list_state.select(None);
            }
        }
    }

    /// Clears the filter and shows all sessions.
    pub fn clear_filter(&mut self) {
        self.search_query.clear();
        self.confirmed_query.clear();
        self.status_filter = None;
        self.filtered_indices = (0..self.sessions.len()).collect();
        self.rebuild_tree_order();
        if !self.filtered_indices.is_empty() {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
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

    /// Applies the current search query and status filter to filter sessions.
    fn apply_filter(&mut self) {
        let query = if self.mode == AppMode::Search {
            &self.search_query
        } else {
            &self.confirmed_query
        };

        let status_filter = self.status_filter;

        self.filtered_indices = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| {
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

        self.rebuild_tree_order();

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

    /// Snapshot of the currently discovered worktree rows, suitable for
    /// driving the clean view's PR fetch. Returns an empty vec while the
    /// discovery is still loading or failed.
    pub fn worktree_rows_snapshot(&self) -> Vec<WorktreeRow> {
        match &self.worktree_view.state {
            super::worktree_view::WorktreeLoadState::Loaded(rows) => rows.clone(),
            _ => Vec::new(),
        }
    }

    /// Switch into the clean view. Records the current view so the user
    /// can return via Esc/n/q, then seeds the row list synchronously
    /// from the worktree snapshot so the user sees rows immediately
    /// while the async PR fetch runs.
    ///
    /// Returns true when the worktree snapshot was non-empty and the
    /// caller should kick off the PR fetch. If false, the clean view
    /// stays in `LoadingPr` and seeding is deferred to
    /// [`Self::seed_clean_view_if_pending`] once worktrees arrive.
    pub fn enter_clean_view(&mut self) -> bool {
        if self.view == View::Clean {
            return false;
        }
        self.clean_return_view = self.view;
        self.view = View::Clean;
        self.clean_view.reset();
        self.seed_clean_view_if_pending()
    }

    /// Seed the clean view from the current worktree snapshot when it
    /// is still waiting for its initial rows. Returns true when seeding
    /// actually happened so the caller can kick off the PR fetch.
    pub fn seed_clean_view_if_pending(&mut self) -> bool {
        if self.view != View::Clean
            || !matches!(
                self.clean_view.state,
                super::clean_view::CleanLoadState::LoadingPr
            )
        {
            return false;
        }
        // Distinguish "discovery still running" from "discovery done
        // with zero worktrees" — the latter must transition out of
        // LoadingPr so the empty-list placeholder renders instead of a
        // permanent "Loading worktrees..." banner.
        let super::worktree_view::WorktreeLoadState::Loaded(rows) = &self.worktree_view.state
        else {
            return false;
        };
        if rows.is_empty() {
            self.clean_view.set_initial_rows(Vec::new());
            self.clean_view.pr_fetch = super::clean_view::PrFetchStatus::Done;
            return false;
        }
        let initial = super::pr_fetch::build_initial_clean_rows(rows.clone(), &self.sessions);
        self.clean_view.set_initial_rows(initial);
        true
    }

    /// Leave the clean view without acting on the partition; returns
    /// to whichever view the user came from.
    pub fn exit_clean_view(&mut self) {
        self.view = self.clean_return_view;
    }

    /// Install fully PR-enriched rows directly. Used by tests; the
    /// production code path goes through [`Self::apply_clean_pr_results`]
    /// instead so the placeholder list set up in `enter_clean_view`
    /// merges with the async result.
    #[cfg(test)]
    pub fn set_clean_rows(&mut self, mut rows: Vec<super::clean_view::CleanRow>) {
        rows = self.filter_already_cleaned(rows);
        self.clean_view.set_rows(rows);
    }

    /// Merge PR-enriched rows returned by the async fetch into the
    /// placeholder list seeded on entry. Drops any path that an
    /// in-flight cleanup has already removed.
    pub fn apply_clean_pr_results(&mut self, rows: Vec<super::clean_view::CleanRow>) {
        let rows = self.filter_already_cleaned(rows);
        self.clean_view.apply_pr_results(rows);
    }

    fn filter_already_cleaned(
        &self,
        mut rows: Vec<super::clean_view::CleanRow>,
    ) -> Vec<super::clean_view::CleanRow> {
        if let Some(progress) = &self.clean_progress {
            let deleted: Vec<PathBuf> = progress
                .confirmed_deleted
                .iter()
                .map(PathBuf::from)
                .collect();
            if !deleted.is_empty() {
                rows.retain(|r| !deleted.iter().any(|d| d == &r.path));
            }
        }
        rows
    }

    /// Mark the PR fetch as failed; the clean view shows the error and
    /// the user can press n/Esc to back out.
    pub fn set_clean_failed(&mut self, error: String) {
        self.clean_view.set_failed(error);
    }

    /// Fold a batch of JSONL events from the detached child into the
    /// live progress state and drop any worktree rows that the child
    /// confirmed deleted.
    pub fn apply_clean_log_events(&mut self, events: &[CleanLogEvent]) {
        let Some(progress) = self.clean_progress.as_mut() else {
            return;
        };
        for event in events {
            progress.apply(event);
        }
        // Drop deleted rows from both lists so the cleanup is reflected
        // without a fresh discovery pass.
        let deleted: Vec<PathBuf> = progress.deleted_paths.iter().map(PathBuf::from).collect();
        if !deleted.is_empty() {
            if let super::worktree_view::WorktreeLoadState::Loaded(rows) =
                &mut self.worktree_view.state
            {
                rows.retain(|r| !deleted.iter().any(|d| d == &r.path));
            }
            self.clean_view.remove_paths(&deleted);
            // Mark the deleted paths as drained so we do not pop the
            // same rows twice on the next batch.
            progress.deleted_paths.clear();
        }
    }

    /// Dismiss the bottom-bar summary. Called on the first key press
    /// after the detached child reports `Done` so the stale "Cleaned
    /// X, failed Y" line does not linger.
    pub fn clear_clean_progress(&mut self) {
        self.clean_progress = None;
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

/// Groups sessions by `cwd` and loads `sessions-index.json` once per project.
/// Returns `cwd` → (`sessionId` → `summary`).
fn load_summaries_by_cwd(sessions: &[Session]) -> HashMap<PathBuf, HashMap<String, String>> {
    let mut by_cwd: HashMap<PathBuf, HashMap<String, String>> = HashMap::new();
    for session in sessions {
        if by_cwd.contains_key(&session.cwd) {
            continue;
        }
        let summaries = claude_sessions::sessions_index_summaries(&session.cwd);
        by_cwd.insert(session.cwd.clone(), summaries);
    }
    by_cwd
}

/// Builds the searchable text cache for all sessions.
fn build_searchable_text_cache(sessions: &[Session]) -> HashMap<String, (String, DateTime<Utc>)> {
    let summaries_by_cwd = load_summaries_by_cwd(sessions);
    sessions
        .iter()
        .map(|session| {
            let searchable_text =
                build_searchable_text_with_summaries(session, summaries_by_cwd.get(&session.cwd));
            (
                session.session_id.clone(),
                (searchable_text, session.updated_at),
            )
        })
        .collect()
}

/// Builds the title cache for all sessions.
fn build_title_cache(sessions: &[Session]) -> HashMap<String, String> {
    let summaries_by_cwd = load_summaries_by_cwd(sessions);
    sessions
        .iter()
        .map(|session| {
            let title =
                get_title_display_name_with_summaries(session, summaries_by_cwd.get(&session.cwd));
            (session.session_id.clone(), title)
        })
        .collect()
}

/// Gets the title display name for a session.
/// Priority: label (armyknife) > sessions-index summary > .jsonl first user prompt > cwd basename.
/// All outputs are sanitized to strip ANSI escape sequences.
fn get_title_display_name(session: &Session) -> String {
    get_title_display_name_with_summaries(session, None)
}

fn get_title_display_name_with_summaries(
    session: &Session,
    summaries: Option<&HashMap<String, String>>,
) -> String {
    // Prefer armyknife's own label (set via env var or auto-generated)
    if let Some(ref label) = session.label {
        return claude_sessions::normalize_title(label);
    }

    if let Some(title) =
        claude_sessions::get_session_title_with_index(&session.cwd, &session.session_id, summaries)
    {
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
    build_searchable_text_with_summaries(session, None)
}

fn build_searchable_text_with_summaries(
    session: &Session,
    summaries: Option<&HashMap<String, String>>,
) -> String {
    let mut parts = Vec::new();

    // tmux session name and window name
    if let Some(ref tmux_info) = session.tmux_info {
        parts.push(tmux_info.session_name.clone());
        parts.push(tmux_info.window_name.clone());
    }

    // Working directory
    parts.push(session.cwd.display().to_string());

    // Claude Code session title
    if let Some(title) =
        claude_sessions::get_session_title_with_index(&session.cwd, &session.session_id, summaries)
    {
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

/// Resolves session labels for the given cwds on the calling thread.
/// Intended for use by a background worker; not called from render.
pub(super) fn resolve_labels_for_cwds(cwds: &[PathBuf]) -> Vec<(PathBuf, String, String)> {
    cwds.iter()
        .map(|cwd| {
            let (repo, worktree) = resolve_session_labels_for_path(cwd);
            (cwd.clone(), repo, worktree)
        })
        .collect()
}

/// Resolves (repo_name, worktree_name) for `cwd` using a single libgit2
/// open. `repo_name` is the main worktree's basename; `worktree_name` is the
/// current branch when resolvable, otherwise the cwd's workdir basename.
/// Falls back to the cwd basename when the path is outside a git repo.
fn resolve_session_labels_for_path(cwd: &Path) -> (String, String) {
    use crate::infra::git::open_repo_at;

    let basename_fallback = || {
        cwd.file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| cwd.display().to_string())
    };

    let Ok(repo) = open_repo_at(cwd) else {
        let fallback = basename_fallback();
        return (fallback.clone(), fallback);
    };

    let repo_name = repo
        .main_workdir()
        .ok()
        .and_then(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
        .unwrap_or_else(basename_fallback);

    let branch = repo.current_branch().ok();
    let worktree_name = branch.filter(|b| b != "HEAD").unwrap_or_else(|| {
        let workdir = repo.workdir();
        workdir
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| workdir.display().to_string())
    });

    (repo_name, worktree_name)
}

/// Resolves the git worktree root for `cwd`. Returns `None` if `cwd` is not
/// inside a repository opened as a worktree (bare main repo or non-git paths
/// are treated as "not a worktree"). The returned path is the worktree's
/// workdir, so matching sibling sessions via `starts_with` is safe even when
/// `cwd` is a subdirectory.
fn resolve_worktree_root(cwd: &Path) -> Option<PathBuf> {
    let repo = crate::infra::git::open_repo_at(cwd).ok()?;
    if !repo.is_worktree() {
        return None;
    }
    Some(repo.workdir().to_path_buf())
}

/// Loads sessions from disk.
///
/// Does not perform stale-session cleanup; that runs once at startup in
/// a background thread (see `EventHandler::new`).
fn load_sessions() -> Result<Vec<Session>> {
    store::list_sessions()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::{SessionStatus, TmuxInfo};
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
            read_at: None,
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
    // session label resolution tests
    // =========================================================================

    #[rstest]
    #[case::normal_path("/home/user/project", "project", "project")]
    #[case::nested_path("/home/user/ghq/github.com/fohte/armyknife", "armyknife", "armyknife")]
    fn test_resolve_session_labels_fallback(
        #[case] cwd: &str,
        #[case] expected_repo: &str,
        #[case] expected_wt: &str,
    ) {
        let (repo, wt) = resolve_session_labels_for_path(&PathBuf::from(cwd));
        assert_eq!(repo, expected_repo);
        assert_eq!(wt, expected_wt);
    }

    #[test]
    fn test_get_cached_worktree_labels_miss_returns_none() {
        let app = create_test_app(vec![]);
        let cwd = PathBuf::from("/home/user/project");
        assert!(app.get_cached_worktree_labels(&cwd).is_none());
    }

    #[test]
    fn test_apply_resolved_labels_populates_cache() {
        let mut app = create_test_app(vec![]);
        let cwd = PathBuf::from("/home/user/project");

        app.apply_resolved_labels(vec![(
            cwd.clone(),
            "project".to_string(),
            "main".to_string(),
        )]);

        assert_eq!(
            app.get_cached_worktree_labels(&cwd),
            Some(("project", "main"))
        );
    }

    #[test]
    fn test_claim_unresolved_label_cwds_dedups_and_marks_pending() {
        let mut app = create_test_app(vec![create_test_session("a"), create_test_session("b")]);
        // Both default sessions share cwd `/tmp/test`, so only one cwd is returned.
        let first = app.claim_unresolved_label_cwds();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0], PathBuf::from("/tmp/test"));

        // Second call returns nothing (already pending).
        let second = app.claim_unresolved_label_cwds();
        assert!(second.is_empty());

        // After the result is applied the cwd is cached and stays cached.
        app.apply_resolved_labels(vec![(
            PathBuf::from("/tmp/test"),
            "test".to_string(),
            "main".to_string(),
        )]);
        let third = app.claim_unresolved_label_cwds();
        assert!(third.is_empty());
        assert_eq!(
            app.get_cached_worktree_labels(&PathBuf::from("/tmp/test")),
            Some(("test", "main"))
        );
    }
}
