use crate::commands::cc::store;
use crate::commands::cc::types::{Session, SessionStatus};
use anyhow::Result;
use chrono::{DateTime, Utc};
use ratatui::widgets::ListState;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::clean_progress::CleanProgress;
use super::clean_view::CleanView;
use super::session_rows::TaskGroup;
use super::worktree_view::WorktreeView;

mod clean;
mod delete;
mod filter;
mod navigation;
mod reload;
mod worktree;

use reload::{build_title_cache, get_title_display_name, load_sessions};
pub(super) use worktree::resolve_labels_for_cwds;

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
    /// If `worktree_cleanup` is `Some`, this is the last session in that
    /// worktree, and confirming will also delete the worktree, its branch,
    /// and associated tmux windows in a single step.
    Confirm {
        session_id: String,
        is_alive: bool,
        worktree_cleanup: Option<PathBuf>,
    },
    /// Editing the title (`label`) of the session with this ID. See
    /// `title_edit` for the enter/update/cancel/confirm orchestration.
    Edit {
        session_id: String,
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
    /// Edit buffer for `AppMode::Edit`, seeded from the session's currently
    /// displayed title when entering edit mode.
    pub edit_title_query: String,
    /// Indices of sessions that match the current filter.
    pub filtered_indices: Vec<usize>,
    /// Selection index before entering search mode (for restoration on cancel).
    pub pre_search_selection: Option<usize>,
    /// Status filter: when set, only sessions with this status are shown.
    pub status_filter: Option<SessionStatus>,
    /// Drill-down scope: when set, the session list shows only this session
    /// (by id) and its descendants (any depth -- same rule as the `▸{n}`
    /// badge). Entered via `App::enter_drilldown`, exited via
    /// `App::exit_drilldown`. Counted as part of `has_filter()`.
    pub drilldown_scope: Option<String>,
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
    /// Maps each display row to the `sessions` index it shows, or `None`
    /// for a row that is not individually selectable (a section header).
    /// Index `i` here always corresponds to
    /// `list_state`'s index `i` and to the `i`-th `ListItem` passed to
    /// `List::new` -- the UI layer must keep those three in exact 1:1
    /// correspondence (no extra `ListItem`s for separators, etc.).
    /// Updated each render by the UI layer after building the row list.
    row_sessions: Vec<Option<usize>>,
    /// Currently active top-level view.
    pub view: View,
    /// Whether the full key-binding list is shown in the help bar (toggled by `?`).
    pub show_help: bool,
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
    /// tq tasks with currently-displayed sessions linked to them. Empty
    /// when tq integration isn't configured, tq is unreachable, or no
    /// displayed session is linked to any task -- the session list falls
    /// back to its flat status-sectioned form in all of those cases.
    pub task_groups: Vec<TaskGroup>,
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
            let old_pos = app.list_state.selected();
            app.restore_selection(old_pos, Some(&session_id));
        }

        Ok(app)
    }

    /// Creates a new App instance with the given sessions.
    /// Useful for testing without disk I/O.
    pub fn with_sessions(sessions: Vec<Session>) -> Self {
        let list_state = ListState::default();

        // Build initial filtered indices (all sessions)
        let filtered_indices: Vec<usize> = (0..sessions.len()).collect();

        // Build title cache for fast UI rendering
        let title_cache = build_title_cache(&sessions);

        let mut app = Self {
            sessions,
            list_state,
            should_quit: false,
            error_message: None,
            mode: AppMode::Normal,
            search_query: String::new(),
            confirmed_query: String::new(),
            edit_title_query: String::new(),
            filtered_indices,
            pre_search_selection: None,
            status_filter: None,
            drilldown_scope: None,
            // Searchable text cache is lazily built on first search
            searchable_text_cache: None,
            row_sessions: Vec::new(),
            title_cache,
            worktree_label_cache: HashMap::new(),
            pending_label_cwds: HashSet::new(),
            view: View::Session,
            show_help: false,
            clean_return_view: View::Session,
            worktree_view: WorktreeView::new(),
            clean_view: CleanView::new(),
            clean_progress: None,
            task_groups: Vec::new(),
        };
        app.rebuild_row_order();
        app.list_state
            .select(app.selectable_positions().first().copied());
        app
    }

    /// Returns the currently selected session, if any.
    pub fn selected_session(&self) -> Option<&Session> {
        self.list_state
            .selected()
            .and_then(|i| self.row_sessions.get(i))
            .and_then(|opt| *opt)
            .and_then(|idx| self.sessions.get(idx))
    }

    /// Returns the filtered sessions for display.
    pub fn filtered_sessions(&self) -> Vec<&Session> {
        self.filtered_indices
            .iter()
            .filter_map(|&i| self.sessions.get(i))
            .collect()
    }

    /// Updates `row_sessions` from display-ordered row session IDs (`None`
    /// for a header/summary row). Called by the UI layer after building
    /// the row list to keep the selection mapping in sync with the
    /// rendered list order.
    pub fn update_row_order(&mut self, row_session_ids: &[Option<&str>]) {
        self.row_sessions = row_session_ids
            .iter()
            .map(|id| id.and_then(|id| self.sessions.iter().position(|s| s.session_id == id)))
            .collect();
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

    /// Toggles whether the full key-binding list is shown in the help bar.
    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    /// Returns the cached title for a session, if available.
    pub fn get_cached_title(&self, session_id: &str) -> Option<&str> {
        self.title_cache.get(session_id).map(String::as_str)
    }

    /// Sets `label` on the given session and recomputes its cached title,
    /// so a rename confirmed in the TUI shows up immediately without
    /// waiting for the file-watcher's own reload.
    pub(crate) fn set_session_label(&mut self, session_id: &str, label: Option<String>) {
        let Some(session) = self
            .sessions
            .iter_mut()
            .find(|s| s.session_id == session_id)
        else {
            return;
        };
        session.label = label;
        let title = get_title_display_name(session);
        self.title_cache.insert(session_id.to_string(), title);
    }

    /// Sets the tq task groups and rebuilds row order so the list re-renders
    /// grouped by task, preserving the current selection the same way a
    /// reload does.
    pub fn set_task_groups(&mut self, task_groups: Vec<TaskGroup>) {
        let selected_session_id = self.selected_session().map(|s| s.session_id.clone());
        let old_pos = self.list_state.selected();
        self.task_groups = task_groups;
        self.restore_selection(old_pos, selected_session_id.as_deref());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;
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

        // Row 0 is the "RUNNING (2)" header; "first"/"second" are rows 1/2.
        app.list_state.select(Some(2));
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
}
