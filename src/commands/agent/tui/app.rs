use crate::commands::agent::store;
#[cfg(test)]
use crate::commands::agent::types::Engine;
use crate::commands::agent::types::{Session, SessionStatus};
use crate::commands::agent::user_turn::next_user_turn;
use crate::shared::config::AgentConfig;
use anyhow::Result;
use chrono::{DateTime, Utc};
use ratatui::widgets::{ListState, TableState};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::clean_progress::CleanProgress;
use super::clean_view::CleanView;
use super::progress::ProgressRow;
use super::session_rows::{SessionTask, build_session_rows_with_vibe, tasks_row_occurrence};
use super::tq_snapshot::TqSnapshot;
use super::worktree::WorktreeDiscoveryState;

use progress::CachedProgress;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct InitialProgressSelection {
    parent_session_id: String,
    delegate_session_id: String,
    select_row: bool,
    expand_batch: bool,
}

impl InitialProgressSelection {
    fn new(parent_session_id: String, delegate_session_id: String) -> Self {
        Self {
            parent_session_id,
            delegate_session_id,
            select_row: true,
            expand_batch: true,
        }
    }
}

struct FocusedTaskAncestor {
    task_session_id: String,
    progress_delegate_id: String,
    task_row_occurrence: usize,
}

mod clean;
mod delete;
mod filter;
mod navigation;
mod progress;
mod progress_cache;
mod progress_scope;
mod progress_selection;
mod reload;
mod tq;
mod worktree;

pub(in crate::commands::agent::tui) use progress_cache::persist_progress_cache;
#[cfg(test)]
pub(super) use progress_scope::ProgressScopeKind;

use reload::{
    SelectionAnchor, build_title_cache, get_title_display_name, load_sessions,
    load_vibe_enabled_session_ids,
};
pub(super) use worktree::resolve_labels_for_cwds;

/// Top-level view selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Session,
    Clean,
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
    /// Makes work type display settings available without reloading config during rendering.
    pub(super) agent_config: AgentConfig,
    /// Session IDs whose stored records explicitly enabled vibe mode.
    pub(super) vibe_enabled_session_ids: HashSet<String>,
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
    /// (by id) and its descendants (any depth). Entered via `App::enter_drilldown`, exited via
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
    /// Worktree discovery shared by Clean view and its background PR fetch.
    pub worktree_discovery: WorktreeDiscoveryState,
    /// Clean-view state (sections, selection, PR-fetch progress).
    pub clean_view: CleanView,
    /// In-flight detached clean progress. `Some` from the moment the
    /// user confirms `y` in the clean view; cleared once the bottom-bar
    /// summary has been on screen long enough for the user to read it.
    pub clean_progress: Option<CleanProgress>,
    /// The first tq task linked to each session in the displayed snapshot,
    /// keyed by session_id. May come from the cache while tq is unavailable.
    pub task_by_session: HashMap<String, SessionTask>,
    /// tq task links used by TASKS rows and tmux pane options.
    pub tq_snapshot: Option<TqSnapshot>,
    /// Whether a tq lookup is currently running.
    pub tq_refreshing: bool,
    /// Whether the session view is using the single-pane layout.
    pub narrow_layout: bool,
    /// Selection and rendered rows for the PROGRESS pane.
    pub(super) progress_list_state: ListState,
    pub(super) progress_table_state: TableState,
    pub(super) progress_rows: Vec<ProgressRow>,
    pub(super) progress_implementation_expanded: bool,
    pub(super) progress_unmatched_expanded: bool,
    pub(super) progress_batch_expanded: HashMap<String, bool>,
    pub(super) progress_session_id: Option<String>,
    pub(super) initial_progress_selection: Option<InitialProgressSelection>,
    pub(super) progress_error: Option<String>,
    progress_cache: HashMap<String, CachedProgress>,
    progress_cache_dirty: bool,
    /// Whether the PROGRESS pane currently receives keyboard input.
    pub progress_focused: bool,
}

impl App {
    pub fn cycle_pane_focus(&mut self) {
        self.progress_focused = !self.progress_focused;
    }

    /// Creates a new App instance with initial session data.
    /// Selects the focused session, the highest-priority user turn, or the
    /// previous selection, in that order.
    ///
    /// `ARMYKNIFE_FOCUS_SESSION` takes precedence over user turns and the
    /// persisted selection.
    pub fn new() -> Result<Self> {
        let sessions = load_sessions()?;
        let vibe_enabled_session_ids = load_vibe_enabled_session_ids()?;
        let agent_config = crate::shared::config::load_config_or_default().agent;

        // Prefer ARMYKNIFE_FOCUS_SESSION over persisted selection
        let focus_session_id = std::env::var("ARMYKNIFE_FOCUS_SESSION")
            .ok()
            .filter(|s| !s.is_empty());
        let last_selected_session_id = store::load_last_selected_session().ok().flatten();

        let mut app = Self::with_initial_selection(
            sessions,
            agent_config,
            focus_session_id.as_deref(),
            last_selected_session_id.as_deref(),
        );
        app.vibe_enabled_session_ids = vibe_enabled_session_ids;
        Ok(app)
    }

    /// Creates a new App instance with the given sessions.
    /// Useful for testing without disk I/O.
    #[cfg(test)]
    pub fn with_sessions(sessions: Vec<Session>) -> Self {
        Self::with_sessions_and_agent_config(sessions, AgentConfig::default())
    }

    /// Shares initialization between startup and tests that inject agent settings.
    pub(super) fn with_sessions_and_agent_config(
        sessions: Vec<Session>,
        agent_config: AgentConfig,
    ) -> Self {
        let list_state = ListState::default();

        // Build initial filtered indices (all sessions)
        let filtered_indices: Vec<usize> = (0..sessions.len()).collect();

        // Build title cache for fast UI rendering
        let title_cache = build_title_cache(&sessions);
        let vibe_enabled_session_ids = sessions
            .iter()
            .filter(|session| session.vibe_enabled)
            .map(|session| session.session_id.clone())
            .collect();

        let mut app = Self {
            sessions,
            agent_config,
            vibe_enabled_session_ids,
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
            worktree_discovery: WorktreeDiscoveryState::default(),
            clean_view: CleanView::new(),
            clean_progress: None,
            task_by_session: HashMap::new(),
            tq_snapshot: None,
            tq_refreshing: false,
            narrow_layout: false,
            progress_list_state: ListState::default(),
            progress_table_state: TableState::default(),
            progress_rows: Vec::new(),
            progress_implementation_expanded: true,
            progress_unmatched_expanded: true,
            progress_batch_expanded: HashMap::new(),
            progress_session_id: None,
            initial_progress_selection: None,
            progress_error: None,
            progress_cache: HashMap::new(),
            progress_cache_dirty: false,
            progress_focused: false,
        };
        app.rebuild_row_order();
        app.list_state
            .select(app.selectable_positions().first().copied());
        app
    }

    fn with_initial_selection(
        sessions: Vec<Session>,
        agent_config: AgentConfig,
        focus_session_id: Option<&str>,
        last_selected_session_id: Option<&str>,
    ) -> Self {
        let mut app = Self::with_sessions_and_agent_config(sessions, agent_config);
        app.restore_initial_selection(focus_session_id, last_selected_session_id);
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

    pub(super) fn selected_session_id(&self) -> Option<&str> {
        self.selected_session()
            .map(|session| session.session_id.as_str())
    }

    fn restore_initial_selection(
        &mut self,
        focus_session_id: Option<&str>,
        last_selected_session_id: Option<&str>,
    ) {
        let focused_session = focus_session_id
            .and_then(|focus_id| self.sessions.iter().find(|s| s.session_id == focus_id));
        if let Some(ancestor) = focus_session_id.and_then(|id| self.focused_task_ancestor(id)) {
            self.progress_focused = true;
            self.initial_progress_selection = Some(InitialProgressSelection::new(
                ancestor.task_session_id.clone(),
                ancestor.progress_delegate_id,
            ));
            self.restore_selection(SelectionAnchor::for_session_occurrence(
                ancestor.task_session_id,
                ancestor.task_row_occurrence,
            ));
            return;
        }

        let selected_session_id = focused_session
            .map(|session| session.session_id.clone())
            .or_else(|| {
                next_user_turn(&self.sessions, &self.agent_config)
                    .map(|turn| turn.session.session_id.clone())
            })
            .or_else(|| last_selected_session_id.map(str::to_owned));

        if let Some(session_id) = selected_session_id {
            self.restore_selection(SelectionAnchor::for_session(session_id));
        }
    }

    fn focused_task_ancestor(&self, focus_session_id: &str) -> Option<FocusedTaskAncestor> {
        let delegate = self
            .sessions
            .iter()
            .find(|session| session.session_id == focus_session_id && session.is_delegated)?;
        let filtered_sessions = self
            .filtered_indices
            .iter()
            .filter_map(|&index| self.sessions.get(index))
            .collect::<Vec<_>>();
        let rows = build_session_rows_with_vibe(
            &filtered_sessions,
            &self.task_by_session,
            &self.agent_config,
            &self.vibe_enabled_session_ids,
        );

        delegate
            .ancestor_session_ids
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, ancestor_id)| {
                let ancestor = self
                    .sessions
                    .iter()
                    .find(|session| session.session_id == *ancestor_id && !session.is_delegated)?;
                let task_row_occurrence = tasks_row_occurrence(&rows, ancestor_id)?;
                let progress_delegate_id = delegate
                    .ancestor_session_ids
                    .get(index + 1)
                    .unwrap_or(&delegate.session_id)
                    .clone();
                Some(FocusedTaskAncestor {
                    task_session_id: ancestor.session_id.clone(),
                    progress_delegate_id,
                    task_row_occurrence,
                })
            })
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::AgentStatus;
    use chrono::{TimeDelta, TimeZone};
    use rstest::{fixture, rstest};
    use std::path::PathBuf;

    #[path = "progress.rs"]
    mod progress;

    #[path = "progress_cache.rs"]
    mod progress_cache;

    /// Counter to assign distinct timestamps to test sessions.
    /// Each call returns a progressively older timestamp, so sessions
    /// created first sort first (most recent updated_at).
    use std::sync::atomic::{AtomicI64, Ordering};
    static TEST_SESSION_COUNTER: AtomicI64 = AtomicI64::new(0);

    fn create_test_session(id: &str) -> Session {
        let offset = TEST_SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let timestamp = Utc::now() - TimeDelta::seconds(offset);
        create_test_session_at(id, timestamp)
    }

    fn create_test_session_at(id: &str, timestamp: chrono::DateTime<Utc>) -> Session {
        Session {
            pr_context: None,
            session_id: id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            is_delegated: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: timestamp,
            updated_at: timestamp,
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

        // Row 0 is the "TASKS" header; "first"/"second" are rows 1/2.
        app.list_state.select(Some(2));
        assert_eq!(
            app.selected_session().map(|s| s.session_id.as_str()),
            Some("second")
        );
    }

    #[fixture]
    fn sessions_with_user_turns() -> Vec<Session> {
        let mut running = create_test_session("running");
        running.status = SessionStatus::Running;
        let mut turn = create_test_session("turn");
        turn.status = SessionStatus::Stopped;
        turn.agent_status = Some(AgentStatus {
            kind: "needs_attention".to_string(),
            note: "choose an option".to_string(),
        });
        let mut earlier_turn = create_test_session("earlier-turn");
        earlier_turn.status = SessionStatus::WaitingInput;
        earlier_turn
            .crit_urls
            .push("https://example.com/review".to_string());

        vec![running, turn, earlier_turn]
    }

    #[rstest]
    #[case::focus_session(Some("running"), Some("earlier-turn"), "running")]
    #[case::first_user_turn(None, Some("running"), "earlier-turn")]
    #[case::stale_focus_falls_through(Some("missing"), Some("running"), "earlier-turn")]
    fn test_initial_selection_prefers_focus_then_user_turn(
        sessions_with_user_turns: Vec<Session>,
        #[case] focus_session_id: Option<&str>,
        #[case] last_selected_session_id: Option<&str>,
        #[case] expected_session_id: &str,
    ) {
        let app = App::with_initial_selection(
            sessions_with_user_turns,
            AgentConfig::default(),
            focus_session_id,
            last_selected_session_id,
        );

        assert_eq!(
            app.selected_session()
                .map(|session| session.session_id.as_str()),
            Some(expected_session_id)
        );
    }

    #[test]
    fn test_initial_selection_restores_fallback_without_user_turns() {
        let first = create_test_session("first");
        let fallback = create_test_session("fallback");
        let app = App::with_initial_selection(
            vec![first, fallback],
            AgentConfig::default(),
            None,
            Some("fallback"),
        );

        assert_eq!(
            app.selected_session()
                .map(|session| session.session_id.as_str()),
            Some("fallback")
        );
    }

    #[test]
    fn delegated_focus_selects_the_nearest_task_ancestor_and_progress_pane() {
        let timestamp = Utc.timestamp_opt(1_800_000_000, 0).single().unwrap();
        let root = create_test_session_at("root", timestamp);
        let mut task = create_test_session_at("task", timestamp);
        task.status = SessionStatus::WaitingInput;
        task.crit_urls
            .push("https://example.test/review".to_string());
        task.ancestor_session_ids = vec![root.session_id.clone()];
        let mut intermediate_delegate = create_test_session_at("intermediate-delegate", timestamp);
        intermediate_delegate.is_delegated = true;
        intermediate_delegate.ancestor_session_ids =
            vec![root.session_id.clone(), task.session_id.clone()];
        let mut delegate = create_test_session_at("delegate", timestamp);
        delegate.is_delegated = true;
        delegate.ancestor_session_ids = vec![
            root.session_id.clone(),
            task.session_id.clone(),
            intermediate_delegate.session_id.clone(),
        ];
        let app = App::with_initial_selection(
            vec![root, task, intermediate_delegate, delegate],
            AgentConfig::default(),
            Some("delegate"),
            None,
        );

        assert_eq!(
            (
                app.selected_session_id(),
                app.selected_session_row_occurrence(),
                app.progress_focused,
                app.initial_progress_selection.clone(),
            ),
            (
                Some("task"),
                Some(1),
                true,
                Some(InitialProgressSelection::new(
                    "task".to_string(),
                    "intermediate-delegate".to_string(),
                )),
            ),
        );
    }

    #[test]
    fn delegated_focus_without_a_task_ancestor_keeps_existing_fallback() {
        let timestamp = Utc.timestamp_opt(1_800_000_000, 0).single().unwrap();
        let first = create_test_session_at("first", timestamp);
        let mut delegate = create_test_session_at("delegate", timestamp);
        delegate.is_delegated = true;
        let app = App::with_initial_selection(
            vec![first, delegate],
            AgentConfig::default(),
            Some("delegate"),
            None,
        );

        assert_eq!(
            (
                app.selected_session_id(),
                app.progress_focused,
                app.initial_progress_selection.clone(),
            ),
            (Some("first"), false, None),
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
