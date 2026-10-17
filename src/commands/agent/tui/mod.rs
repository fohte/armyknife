mod app;
mod clean_progress;
mod clean_view;
mod crit_key;
mod event;
mod pr_fetch;
mod progress;
mod progress_batches;
mod progress_fetch;
mod progress_key;
mod progress_refresh;
mod session_key;
mod session_preview;
mod session_rows;
mod title_edit;
mod title_generate;
mod tq_cache;
mod tq_fetch;
mod tq_pane_options;
mod tq_snapshot;
mod ui;
mod worktree;
mod worktree_session_children;

#[cfg(test)]
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::DefaultTerminal;

use self::app::App;
#[cfg(test)]
use self::event::KeyEvent;
use self::event::{AppEvent, EventHandler};
use self::progress::ProgressOpenTarget;
use self::progress_refresh::{
    ProgressRefreshSchedule, apply_tq_snapshot_and_schedule_progress_refresh,
    start_pending_progress_refresh, start_progress_refresh,
};
use self::session_key::handle_key_event;
use self::session_preview::PreviewError;
use crate::shared::command;
#[cfg(test)]
use crossterm::event::{KeyCode, KeyModifiers};

const TQ_SESSION_TASKS_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

struct TqRefreshSchedule {
    in_flight: bool,
    pending: bool,
    first_snapshot_applied: bool,
    next_periodic_refresh: Instant,
}

impl TqRefreshSchedule {
    fn new(now: Instant) -> Self {
        Self {
            in_flight: true,
            pending: false,
            first_snapshot_applied: false,
            next_periodic_refresh: now + TQ_SESSION_TASKS_REFRESH_INTERVAL,
        }
    }

    fn periodic_refresh_due(&mut self, now: Instant) -> bool {
        if now < self.next_periodic_refresh {
            return false;
        }
        self.next_periodic_refresh = now + TQ_SESSION_TASKS_REFRESH_INTERVAL;
        true
    }

    fn fetch_completed(&mut self) {
        self.in_flight = false;
    }

    fn mark_snapshot_applied(&mut self) -> bool {
        !std::mem::replace(&mut self.first_snapshot_applied, true)
    }

    fn request(&mut self) {
        self.pending = true;
    }

    fn start_pending(&mut self) -> bool {
        if self.in_flight || !self.pending {
            return false;
        }
        self.in_flight = true;
        self.pending = false;
        true
    }
}

/// Runs the TUI application.
pub fn run() -> Result<()> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal);
    ratatui::restore();
    let crit_session_id = result?;
    if let Some(session_id) = crit_session_id {
        super::crit::spawn_open_after_watch(&session_id)?;
    }
    Ok(())
}

/// Side effects requested by key handlers that need access to the event
/// handler (to spawn background work). Returning these from the pure
/// handlers keeps them testable without dragging in real async / IO.
#[derive(Debug, Default, PartialEq, Eq)]
struct KeyEffects {
    /// User pressed `c`: kick off the PR fetch for the clean view.
    request_clean_pr_fetch: bool,
    /// User confirmed `y` in the clean view: spawn the detached child
    /// for these paths and start tailing its log.
    spawn_detached_clean: Option<Vec<PathBuf>>,
    /// User pressed `p`: open the selected session's JSONL in
    /// `claude-history` after suspending the TUI.
    preview_session_path: Option<PathBuf>,
    /// User pressed Ctrl+g in title-edit mode: spawn the detached
    /// title-generation process for this request.
    spawn_title_generation: Option<title_generate::SpawnTitleGenerationRequest>,
    /// User pressed `t`: fetch this task's web URL from `tq` (its result
    /// opens the browser once `AppEvent::TaskUrlFetched` arrives).
    fetch_task_url: Option<String>,
    /// User pressed `o`: leave watch and open this session's crit pane.
    open_crit_session_id: Option<String>,
    /// User pressed Enter on a PROGRESS pane row.
    open_progress_target: Option<ProgressOpenTarget>,
}

impl KeyEffects {
    fn merge(&mut self, other: KeyEffects) {
        if other.request_clean_pr_fetch {
            self.request_clean_pr_fetch = true;
        }
        if other.spawn_detached_clean.is_some() {
            self.spawn_detached_clean = other.spawn_detached_clean;
        }
        if other.preview_session_path.is_some() {
            self.preview_session_path = other.preview_session_path;
        }
        if other.spawn_title_generation.is_some() {
            self.spawn_title_generation = other.spawn_title_generation;
        }
        if other.fetch_task_url.is_some() {
            self.fetch_task_url = other.fetch_task_url;
        }
        if other.open_crit_session_id.is_some() {
            self.open_crit_session_id = other.open_crit_session_id;
        }
        if other.open_progress_target.is_some() {
            self.open_progress_target = other.open_progress_target;
        }
    }
}

/// Maximum events to drain per iteration to prevent starvation under sustained load.
const MAX_DRAIN_PER_ITERATION: usize = 100;

/// Main application loop.
///
/// Uses an event-drain strategy to prevent queue buildup:
/// 1. Block-wait for the first event
/// 2. Drain remaining queued events (non-blocking, up to MAX_DRAIN_PER_ITERATION)
/// 3. Key events are processed immediately during drain
/// 4. Session-store changes are merged by database revision
/// 5. The reload + render happens once per iteration
fn run_app(terminal: &mut DefaultTerminal) -> Result<Option<String>> {
    let mut app = App::new()?;
    if let Err(error) = app.load_progress_cache_from_disk() {
        tracing::warn!("failed to load progress cache: {error}");
    }
    match tq_cache::load() {
        Ok(Some(snapshot)) => {
            app.set_tq_snapshot(snapshot);
        }
        Ok(None) => {}
        Err(error) => tracing::warn!("failed to load tq task cache: {error}"),
    }
    let event_handler = EventHandler::new()?;
    app.reload_session_store_changes()?;
    let mut crit_session_id = None;

    let mut progress_refresh_schedule = ProgressRefreshSchedule::new(Instant::now());
    progress_refresh_schedule.request(app.selected_session_id().map(str::to_owned));

    let local_session_ids: HashSet<String> =
        app.sessions.iter().map(|s| s.session_id.clone()).collect();
    app.tq_refresh_started();
    event_handler.start_tq_task_fetch(local_session_ids);
    let mut tq_refresh_schedule = TqRefreshSchedule::new(Instant::now());
    start_pending_progress_refresh(
        &mut progress_refresh_schedule,
        Instant::now(),
        |session_id| start_progress_refresh(&mut app, &event_handler, session_id),
    );

    loop {
        let unresolved = app.claim_unresolved_label_cwds();
        if !unresolved.is_empty() {
            event_handler.start_session_labels_resolve(unresolved);
        }

        terminal.draw(|frame| ui::render(frame, &mut app))?;

        let first_event = event_handler.next()?;
        let selected_session_before_events = app.selected_session_id().map(str::to_owned);
        let mut session_store_changed = false;
        let mut periodic_tq_refresh_due = false;
        let mut periodic_progress_refresh_due = false;
        let mut effects = KeyEffects::default();

        // Process first event + drain queued events (bounded to prevent starvation)
        for event in std::iter::once(first_event)
            .chain(std::iter::from_fn(|| event_handler.try_next()))
            .take(MAX_DRAIN_PER_ITERATION)
        {
            match event {
                AppEvent::Key(k) => effects.merge(handle_key_event(&mut app, k)),
                AppEvent::SessionStoreChanged => session_store_changed = true,
                AppEvent::Tick => {
                    if tq_refresh_schedule.periodic_refresh_due(Instant::now()) {
                        periodic_tq_refresh_due = true;
                    }
                    if progress_refresh_schedule.periodic_refresh_due(Instant::now()) {
                        periodic_progress_refresh_due = true;
                    }
                }
                AppEvent::WorktreesLoaded(Ok(rows)) => {
                    app.set_worktrees(rows);
                    // If the user opened the clean view before discovery
                    // finished, seed it now and kick off the PR fetch.
                    if app.seed_clean_view_if_pending() {
                        effects.request_clean_pr_fetch = true;
                    }
                }
                AppEvent::WorktreesLoaded(Err(err)) => {
                    app.set_worktrees_failed(err);
                }
                AppEvent::SessionLabelsResolved(results) => {
                    app.apply_resolved_labels(results);
                }
                AppEvent::CleanPrFetched(Ok(rows)) => {
                    app.apply_clean_pr_results(rows);
                }
                AppEvent::CleanPrFetched(Err(err)) => {
                    app.set_clean_failed(err);
                }
                AppEvent::ProgressFetched { session_id, result } => {
                    progress_refresh_schedule.fetch_completed();
                    app.apply_progress_result(&session_id, result);
                }
                AppEvent::CleanLogEvents(events) => {
                    app.apply_clean_log_events(&events);
                }
                AppEvent::TqTasksFetched(Ok(Some(snapshot))) => {
                    tq_refresh_schedule.fetch_completed();
                    if let Err(error) = tq_cache::store(&snapshot) {
                        tracing::warn!("failed to store tq task cache: {error}");
                    }
                    apply_tq_snapshot_and_schedule_progress_refresh(
                        &mut app,
                        &mut progress_refresh_schedule,
                        snapshot,
                    );
                    if let Some(snapshot) = app.tq_snapshot.as_ref() {
                        tq_pane_options::sync(snapshot);
                    }
                    app.tq_refresh_finished();
                    schedule_progress_after_first_tq_snapshot(
                        &app,
                        &mut tq_refresh_schedule,
                        &mut progress_refresh_schedule,
                    );
                }
                AppEvent::TqTasksFetched(Ok(None)) => {
                    tq_refresh_schedule.fetch_completed();
                    app.tq_refresh_finished();
                }
                AppEvent::TqTasksFetched(Err(e)) => {
                    tq_refresh_schedule.fetch_completed();
                    app.tq_refresh_finished();
                    tracing::warn!("tq task fetch failed: {e}");
                }
                AppEvent::TaskUrlFetched(Ok(url)) => {
                    open_url(&mut app, &url);
                }
                AppEvent::TaskUrlFetched(Err(e)) => {
                    tracing::warn!("tq task-url fetch failed: {e}");
                }
            }
        }

        if effects.request_clean_pr_fetch {
            let rows = app.worktree_rows_snapshot();
            let sessions = app.sessions.clone();
            event_handler.start_clean_pr_fetch(rows, sessions);
        }
        if let Some(path) = effects.preview_session_path {
            match session_preview::preview_session(terminal, &path) {
                Ok(()) => {}
                Err(PreviewError::Viewer(e)) => {
                    app.set_error(format!("Failed to preview session: {e}"));
                }
                // A terminal that will not restore is not something we can
                // paper over — bail out so `ratatui::restore()` runs and
                // the shell gets a chance to clean up.
                Err(PreviewError::Fatal(e)) => return Err(e),
            }
        }
        if let Some(paths) = effects.spawn_detached_clean {
            match clean_progress::spawn_detached_clean(&paths) {
                Ok(run_id) => {
                    app.clean_progress = Some(clean_progress::CleanProgress::new(run_id.clone()));
                    if let Some(log_path) = clean_progress::live_log_path() {
                        event_handler.start_clean_tail(log_path, run_id);
                    }
                }
                Err(e) => {
                    app.set_error(format!("Failed to spawn cleanup: {e}"));
                }
            }
        }
        if let Some(request) = effects.spawn_title_generation
            && let Err(e) = title_generate::spawn_detached_title_generation(request)
        {
            app.set_error(format!("Failed to start title generation: {e}"));
        }
        if let Some(task_id) = effects.fetch_task_url {
            event_handler.start_task_url_fetch(task_id);
        }
        if effects.open_crit_session_id.is_some() {
            crit_session_id = effects.open_crit_session_id;
        }
        if let Some(target) = effects.open_progress_target {
            match target {
                ProgressOpenTarget::FocusPane(pane_id) => {
                    if let Err(error) = crate::infra::tmux::focus_pane(&pane_id) {
                        app.set_error(format!("Failed to focus pane: {error}"));
                    }
                }
                ProgressOpenTarget::OpenUrl(url) => open_url(&mut app, &url),
            }
        }

        // Apply merged session changes in a single reload
        let mut sessions_changed = false;
        let mut new_session_created = false;
        if session_store_changed {
            let (changed, created) = app.reload_session_store_changes()?;
            sessions_changed |= changed;
            new_session_created |= created;
        }
        if sessions_changed {
            app.refresh_worktree_session_counts();
            if let Some(session_id) = app.selected_session_id().map(str::to_owned)
                && app.progress_session_id.as_deref() == Some(session_id.as_str())
            {
                app.refresh_progress_from_sessions(&session_id);
                if progress_refresh_schedule.in_flight {
                    progress_refresh_schedule.request(Some(session_id));
                }
            }
        }
        let selected_session_after_events = app.selected_session_id().map(str::to_owned);
        let selection_changed = selected_session_before_events != selected_session_after_events;
        if selection_changed || periodic_progress_refresh_due {
            if selection_changed
                && progress_refresh_schedule.in_flight
                && let Some(session_id) = selected_session_after_events.as_deref()
            {
                app.refresh_progress_from_sessions(session_id);
            }
            progress_refresh_schedule.request(selected_session_after_events.clone());
        }
        // A new session may already be tq-linked; refresh its prefix along
        // with periodic refreshes, coalescing requests while one is running.
        if new_session_created || periodic_tq_refresh_due {
            tq_refresh_schedule.request();
        }
        if tq_refresh_schedule.start_pending() {
            let local_session_ids: HashSet<String> =
                app.sessions.iter().map(|s| s.session_id.clone()).collect();
            app.tq_refresh_started();
            event_handler.start_tq_task_fetch(local_session_ids);
        }

        start_pending_progress_refresh(
            &mut progress_refresh_schedule,
            Instant::now(),
            |session_id| start_progress_refresh(&mut app, &event_handler, session_id),
        );
        app::persist_progress_cache(&mut app);

        if app.should_quit {
            break;
        }
    }

    Ok(crit_session_id)
}

/// Opens `url` in the browser via the platform's default opener. Unlike a
/// failed `tq` fetch (silently ignored -- see `AppEvent::TaskUrlFetched`),
/// a failure here is unexpected and surfaced as an error banner.
fn open_url(app: &mut App, url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(e) = command::new(opener).arg(url).spawn() {
        app.set_error(format!("Failed to open URL: {e}"));
    }
}

fn schedule_progress_after_first_tq_snapshot(
    app: &App,
    tq_refresh_schedule: &mut TqRefreshSchedule,
    progress_refresh_schedule: &mut ProgressRefreshSchedule,
) {
    if tq_refresh_schedule.mark_snapshot_applied() {
        progress_refresh_schedule.request(app.selected_session_id().map(str::to_owned));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::tui::app::AppMode;
    use crate::commands::agent::types::{Engine, Session, SessionStatus};
    use chrono::Utc;
    use rstest::{fixture, rstest};
    use std::path::PathBuf;

    fn create_test_app_with_sessions(count: usize) -> App {
        let now = Utc::now();
        let sessions: Vec<Session> = (0..count)
            .map(|i| Session {
                archived_at: None,
                pr_context: None,
                session_id: format!("session-{}", i),
                work_type: None,
                work_type_pinned: false,
                vibe_enabled: false,
                launched_with_vibe: false,
                is_delegated: false,
                crit_urls: Vec::new(),
                pending_human_review_ids: Default::default(),
                cwd: PathBuf::from(format!("/project/{}", i)),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Running,
                created_at: now,
                updated_at: now,
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
            })
            .collect();

        App::with_sessions(sessions)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn key_ctrl(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
        }
    }

    fn type_string(app: &mut App, s: &str) {
        for c in s.chars() {
            handle_key_event(app, key(KeyCode::Char(c)));
        }
    }

    #[rstest]
    #[case::q(KeyCode::Char('q'))]
    #[case::esc(KeyCode::Esc)]
    fn test_handle_key_quit(#[case] code: KeyCode) {
        let mut app = create_test_app_with_sessions(1);
        handle_key_event(&mut app, key(code));
        assert!(app.should_quit);
    }

    #[test]
    fn test_handle_key_toggle_help() {
        let mut app = create_test_app_with_sessions(1);
        assert!(!app.show_help);
        handle_key_event(&mut app, key(KeyCode::Char('?')));
        assert!(app.show_help);
        handle_key_event(&mut app, key(KeyCode::Char('?')));
        assert!(!app.show_help);
    }

    #[rstest]
    #[case::j(KeyCode::Char('j'), Some(1), Some(2))]
    #[case::k(KeyCode::Char('k'), Some(3), Some(2))]
    #[case::down(KeyCode::Down, Some(1), Some(2))]
    #[case::up(KeyCode::Up, Some(2), Some(1))]
    fn test_handle_key_navigation(
        #[case] code: KeyCode,
        #[case] initial: Option<usize>,
        #[case] expected: Option<usize>,
    ) {
        let mut app = create_test_app_with_sessions(3);
        app.list_state.select(initial);
        handle_key_event(&mut app, key(code));
        assert_eq!(app.list_state.selected(), expected);
    }

    // Row 0 is the "TASKS" header; sessions are rows 1/2.
    #[rstest]
    #[case::h(KeyCode::Char('h'))]
    #[case::left(KeyCode::Left)]
    fn test_handle_key_parent_navigation(#[case] code: KeyCode) {
        let mut app = create_test_app_with_sessions(2);
        app.sessions[1].ancestor_session_ids = vec!["session-0".to_string()];
        app.list_state.select(Some(2));

        handle_key_event(&mut app, key(code));

        assert_eq!(app.list_state.selected(), Some(1));
    }

    // Row 0 is the "TASKS" header; sessions are rows 1/2.
    #[rstest]
    #[case::l(KeyCode::Char('l'))]
    #[case::right(KeyCode::Right)]
    fn test_handle_key_drilldown_navigation(#[case] code: KeyCode) {
        let mut app = create_test_app_with_sessions(2);
        app.sessions[1].ancestor_session_ids = vec!["session-0".to_string()];
        app.list_state.select(Some(1)); // session-0, which has a descendant

        handle_key_event(&mut app, key(code));

        assert_eq!(app.drilldown_scope, Some("session-0".to_string()));
    }

    #[rstest]
    #[case::l(KeyCode::Char('l'))]
    #[case::right(KeyCode::Right)]
    fn test_handle_key_drilldown_navigation_noop_without_descendants(#[case] code: KeyCode) {
        let mut app = create_test_app_with_sessions(2);
        app.list_state.select(Some(1));

        handle_key_event(&mut app, key(code));

        assert_eq!(app.drilldown_scope, None);
    }

    // All 5 sessions default to `Running`, so row 0 is the "TASKS"
    // header and rows 1-5 are the sessions.
    #[rstest]
    #[case::select_3('3', Some(1), Some(3))]
    #[case::select_1('1', Some(3), Some(1))]
    #[case::zero_ignored('0', Some(2), Some(2))]
    fn test_handle_key_quick_select(
        #[case] c: char,
        #[case] initial: Option<usize>,
        #[case] expected: Option<usize>,
    ) {
        let mut app = create_test_app_with_sessions(5);
        app.list_state.select(initial);
        handle_key_event(&mut app, key(KeyCode::Char(c)));
        assert_eq!(app.list_state.selected(), expected);
    }

    // =========================================================================
    // Search mode tests
    // =========================================================================

    #[test]
    fn test_enter_search_mode_with_slash() {
        let mut app = create_test_app_with_sessions(3);
        assert_eq!(app.mode, AppMode::Normal);

        handle_key_event(&mut app, key(KeyCode::Char('/')));
        assert_eq!(app.mode, AppMode::Search);
    }

    #[test]
    fn test_search_mode_typing() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "test");
        assert_eq!(app.search_query, "test");
    }

    #[test]
    fn test_search_mode_jk_are_typed_not_navigation() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "jk");
        // j and k should be typed as characters, not used for navigation
        assert_eq!(app.search_query, "jk");
    }

    #[test]
    fn test_search_mode_ctrl_n_p_for_navigation() {
        let mut app = create_test_app_with_sessions(3);
        // Row 0 is the "TASKS" header, so the default selection
        // (first selectable row) already sits on row 1.
        assert_eq!(app.list_state.selected(), Some(1));
        handle_key_event(&mut app, key(KeyCode::Char('/')));

        // Ctrl+n should move down
        handle_key_event(&mut app, key_ctrl('n'));
        assert_eq!(app.list_state.selected(), Some(2));

        // Ctrl+p should move up
        handle_key_event(&mut app, key_ctrl('p'));
        assert_eq!(app.list_state.selected(), Some(1));
    }

    #[test]
    fn test_search_mode_backspace() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "tes");
        handle_key_event(&mut app, key(KeyCode::Backspace));
        assert_eq!(app.search_query, "te");
    }

    #[test]
    fn test_search_mode_ctrl_u_clears_query() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "test");
        handle_key_event(&mut app, key_ctrl('u'));
        assert_eq!(app.search_query, "");
    }

    #[test]
    fn test_search_mode_ctrl_w_deletes_word() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "hello world");
        handle_key_event(&mut app, key_ctrl('w'));
        assert_eq!(app.search_query, "hello ");
    }

    #[test]
    fn test_search_mode_confirm() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "0");
        handle_key_event(&mut app, key(KeyCode::Enter));

        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.confirmed_query, "0");
        assert!(app.has_filter());
    }

    #[test]
    fn test_search_mode_cancel() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "te");
        handle_key_event(&mut app, key(KeyCode::Esc));

        assert_eq!(app.mode, AppMode::Normal);
        assert!(!app.has_filter());
    }

    #[test]
    fn test_esc_clears_filter_in_normal_mode() {
        let mut app = create_test_app_with_sessions(3);

        handle_key_event(&mut app, key(KeyCode::Char('/')));
        type_string(&mut app, "0");
        handle_key_event(&mut app, key(KeyCode::Enter));
        assert!(app.has_filter());

        handle_key_event(&mut app, key(KeyCode::Esc));
        assert!(!app.has_filter());
        assert!(!app.should_quit);
    }

    #[test]
    fn test_esc_quits_when_no_filter() {
        let mut app = create_test_app_with_sessions(3);
        assert!(!app.has_filter());

        handle_key_event(&mut app, key(KeyCode::Esc));
        assert!(app.should_quit);
    }

    // =========================================================================
    // Preview key binding tests
    // =========================================================================

    #[rstest]
    #[case::with_transcript(
        1,
        Some(PathBuf::from("/tmp/session-0.jsonl")),
        key(KeyCode::Char('p')),
        KeyEffects {
            preview_session_path: Some(PathBuf::from("/tmp/session-0.jsonl")),
            ..Default::default()
        },
    )]
    #[case::no_transcript(1, None, key(KeyCode::Char('p')), KeyEffects::default())]
    #[case::no_selection(0, None, key(KeyCode::Char('p')), KeyEffects::default())]
    // Ctrl+p still toggles the Paused filter — plain `p` must not shadow it.
    #[case::ctrl_p_filter(
        1,
        Some(PathBuf::from("/tmp/x.jsonl")),
        key_ctrl('p'),
        KeyEffects::default()
    )]
    fn test_preview_key_effects(
        #[case] session_count: usize,
        #[case] transcript_path: Option<PathBuf>,
        #[case] key: KeyEvent,
        #[case] expected: KeyEffects,
    ) {
        let mut app = create_test_app_with_sessions(session_count);
        if let Some(path) = transcript_path
            && !app.sessions.is_empty()
        {
            app.sessions[0].transcript_path = Some(path);
        }
        assert_eq!(handle_key_event(&mut app, key), expected);
    }

    #[test]
    fn test_ctrl_p_toggles_paused_filter_even_with_p_binding() {
        // Regression guard: plain `p` intercepts before the Ctrl+p filter
        // path, so verify Ctrl+p still reaches it end-to-end.
        let mut app = create_test_app_with_sessions(1);
        handle_key_event(&mut app, key_ctrl('p'));
        assert_eq!(app.status_filter, Some(SessionStatus::Paused));
    }

    // =========================================================================
    // Open task key binding tests
    // =========================================================================

    fn test_task(task_id: &str) -> super::session_rows::SessionTask {
        super::session_rows::SessionTask {
            task_id: task_id.to_string(),
            task_number: 1,
            task_title: "Test task".to_string(),
            parent_task_id: None,
            is_closed: false,
        }
    }

    #[rstest]
    #[case::with_linked_task(
        1,
        Some(test_task("task-1")),
        KeyEffects {
            fetch_task_url: Some("task-1".to_string()),
            ..Default::default()
        },
    )]
    #[case::no_linked_task(1, None, KeyEffects::default())]
    #[case::no_selection(0, None, KeyEffects::default())]
    fn test_open_task_key_effects(
        #[case] session_count: usize,
        #[case] task: Option<super::session_rows::SessionTask>,
        #[case] expected: KeyEffects,
    ) {
        let mut app = create_test_app_with_sessions(session_count);
        if let Some(task) = task {
            app.task_by_session.insert("session-0".to_string(), task);
        }
        assert_eq!(
            handle_key_event(&mut app, key(KeyCode::Char('t'))),
            expected
        );
    }

    #[test]
    fn test_successful_empty_tq_snapshot_clears_previous_task_links() {
        let mut app = create_test_app_with_sessions(1);
        app.set_tq_snapshot(super::tq_snapshot::TqSnapshot::new(HashMap::from([(
            "session-0".to_string(),
            vec![test_task("task-1")],
        )])));

        assert_eq!(
            app.task_by_session,
            HashMap::from([("session-0".to_string(), test_task("task-1"))]),
        );

        app.set_tq_snapshot(super::tq_snapshot::TqSnapshot::new(HashMap::new()));

        assert_eq!(app.task_by_session, HashMap::new());
    }

    #[test]
    fn test_first_tq_snapshot_queues_progress_refresh_after_initial_fetch() {
        let now = Instant::now();
        let mut app = create_test_app_with_sessions(1);
        let mut tq_refresh_schedule = TqRefreshSchedule::new(now);
        let mut progress_refresh_schedule = ProgressRefreshSchedule::new(now);

        progress_refresh_schedule.request(Some("session-0".to_string()));
        let _ = progress_refresh_schedule.start_pending();

        tq_refresh_schedule.fetch_completed();
        app.set_tq_snapshot(super::tq_snapshot::TqSnapshot::new(HashMap::from([(
            "session-0".to_string(),
            vec![test_task("task-1")],
        )])));
        schedule_progress_after_first_tq_snapshot(
            &app,
            &mut tq_refresh_schedule,
            &mut progress_refresh_schedule,
        );
        progress_refresh_schedule.fetch_completed();
        let first_refresh = progress_refresh_schedule.start_pending();

        app.set_tq_snapshot(super::tq_snapshot::TqSnapshot::new(HashMap::from([(
            "session-0".to_string(),
            vec![test_task("task-1")],
        )])));
        schedule_progress_after_first_tq_snapshot(
            &app,
            &mut tq_refresh_schedule,
            &mut progress_refresh_schedule,
        );
        progress_refresh_schedule.fetch_completed();
        let repeated_refresh = progress_refresh_schedule.start_pending();

        assert_eq!(
            (first_refresh, repeated_refresh),
            (Some("session-0".to_string()), None),
        );
    }

    #[test]
    fn tq_scope_change_queues_refresh_after_an_in_flight_fetch() {
        let now = Instant::now();
        let mut app = create_test_app_with_sessions(2);
        app.list_state.select(Some(1));
        let _ = app.set_tq_snapshot(super::tq_snapshot::TqSnapshot::new(HashMap::from([(
            "session-0".to_string(),
            vec![test_task("task-1")],
        )])));
        let mut schedule = ProgressRefreshSchedule::new(now);
        schedule.request(Some("session-0".to_string()));
        assert_eq!(schedule.start_pending(), Some("session-0".to_string()));

        let scope_changed = apply_tq_snapshot_and_schedule_progress_refresh(
            &mut app,
            &mut schedule,
            super::tq_snapshot::TqSnapshot::new(HashMap::from([
                ("session-0".to_string(), vec![test_task("task-1")]),
                ("session-1".to_string(), vec![test_task("task-1")]),
            ])),
        );
        let next_scope = app.progress_scope_for_session("session-0");
        schedule.fetch_completed();
        let next_refresh = schedule.start_pending();

        assert_eq!(
            (scope_changed, next_scope, next_refresh),
            (
                true,
                Some(super::app::ProgressScopeKind::Task {
                    task_id: "task-1".to_string(),
                    session_ids: vec!["session-0".to_string(), "session-1".to_string()],
                }),
                Some("session-0".to_string()),
            ),
        );
    }

    #[test]
    fn skipped_progress_fetch_leaves_schedule_ready() {
        let now = Instant::now();
        let mut schedule = ProgressRefreshSchedule::new(now);
        schedule.request(Some("project-manager".to_string()));
        start_pending_progress_refresh(&mut schedule, now, |_| {
            super::progress_refresh::ProgressRefreshStart::MissingScope
        });
        let skipped_state = (schedule.in_flight, schedule.pending_session_id.clone());

        schedule.request(Some("session-0".to_string()));
        start_pending_progress_refresh(&mut schedule, now, |_| {
            super::progress_refresh::ProgressRefreshStart::Started
        });

        assert_eq!(
            (
                skipped_state,
                schedule.in_flight,
                schedule.pending_session_id
            ),
            ((false, None), true, None),
        );
    }

    #[test]
    fn fresh_progress_attempt_schedules_periodic_refresh_at_its_due_time() {
        let now = Instant::now();
        let mut schedule = ProgressRefreshSchedule::new(now);
        let periodic_due = now + Duration::from_secs(30);
        schedule.request(Some("session-0".to_string()));
        start_pending_progress_refresh(&mut schedule, periodic_due, |_| {
            super::progress_refresh::ProgressRefreshStart::Fresh(Duration::from_secs(20))
        });

        assert_eq!(
            (
                schedule.periodic_refresh_due(periodic_due + Duration::from_secs(19)),
                schedule.periodic_refresh_due(periodic_due + Duration::from_secs(20)),
            ),
            (false, true),
        );
    }

    // =========================================================================
    // =========================================================================
    // Status filter key binding tests
    // =========================================================================

    #[fixture]
    fn app_with_statuses() -> App {
        let sessions: Vec<Session> = vec![
            Session {
                archived_at: None,
                pr_context: None,
                session_id: "session-running".to_string(),
                work_type: None,
                work_type_pinned: false,
                vibe_enabled: false,
                launched_with_vibe: false,
                is_delegated: false,
                crit_urls: Vec::new(),
                pending_human_review_ids: Default::default(),
                cwd: PathBuf::from("/project/running"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Running,
                created_at: Utc::now(),
                updated_at: Utc::now(),
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
            },
            Session {
                archived_at: None,
                pr_context: None,
                session_id: "session-waiting".to_string(),
                work_type: None,
                work_type_pinned: false,
                vibe_enabled: false,
                launched_with_vibe: false,
                is_delegated: false,
                crit_urls: Vec::new(),
                pending_human_review_ids: Default::default(),
                cwd: PathBuf::from("/project/waiting"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::WaitingInput,
                created_at: Utc::now(),
                updated_at: Utc::now(),
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
            },
            Session {
                archived_at: None,
                pr_context: None,
                session_id: "session-stopped".to_string(),
                work_type: None,
                work_type_pinned: false,
                vibe_enabled: false,
                launched_with_vibe: false,
                is_delegated: false,
                crit_urls: Vec::new(),
                pending_human_review_ids: Default::default(),
                cwd: PathBuf::from("/project/stopped"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Stopped,
                created_at: Utc::now(),
                updated_at: Utc::now(),
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
            },
            Session {
                archived_at: None,
                pr_context: None,
                session_id: "session-paused".to_string(),
                work_type: None,
                work_type_pinned: false,
                vibe_enabled: false,
                launched_with_vibe: false,
                is_delegated: false,
                crit_urls: Vec::new(),
                pending_human_review_ids: Default::default(),
                cwd: PathBuf::from("/project/paused"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Paused,
                created_at: Utc::now(),
                updated_at: Utc::now(),
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
            },
        ];
        App::with_sessions(sessions)
    }

    #[rstest]
    #[case::ctrl_w_toggles_waiting('w', SessionStatus::WaitingInput)]
    #[case::ctrl_s_toggles_stopped('s', SessionStatus::Stopped)]
    #[case::ctrl_r_toggles_running('r', SessionStatus::Running)]
    #[case::ctrl_p_toggles_paused('p', SessionStatus::Paused)]
    fn test_handle_key_status_filter(
        app_with_statuses: App,
        #[case] c: char,
        #[case] expected_status: SessionStatus,
    ) {
        let mut app = app_with_statuses;
        handle_key_event(&mut app, key_ctrl(c));

        assert_eq!(app.status_filter, Some(expected_status));
        // All filtered sessions should have the expected status
        for session in app.filtered_sessions() {
            assert_eq!(session.status, expected_status);
        }
    }

    #[rstest]
    #[case::plain_w('w')]
    #[case::plain_s('s')]
    #[case::plain_r('r')]
    fn test_plain_wsr_do_not_filter(app_with_statuses: App, #[case] c: char) {
        // Without Ctrl, these keys should not activate a status filter -- they
        // are reserved for future actions (e.g., `r` = resume).
        let mut app = app_with_statuses;
        handle_key_event(&mut app, key(KeyCode::Char(c)));
        assert!(app.status_filter.is_none());
    }

    #[rstest]
    fn test_status_filter_toggle_off(app_with_statuses: App) {
        let mut app = app_with_statuses;

        // Press Ctrl+w to set WaitingInput filter
        handle_key_event(&mut app, key_ctrl('w'));
        assert_eq!(app.status_filter, Some(SessionStatus::WaitingInput));
        assert_eq!(app.filtered_sessions().len(), 1);

        // Press Ctrl+w again to clear the filter
        handle_key_event(&mut app, key_ctrl('w'));
        assert!(app.status_filter.is_none());
        assert_eq!(app.filtered_sessions().len(), 4);
    }

    #[rstest]
    fn test_esc_clears_status_filter(app_with_statuses: App) {
        let mut app = app_with_statuses;

        // Set status filter
        handle_key_event(&mut app, key_ctrl('w'));
        assert!(app.has_filter());

        // Press Esc to clear filter (should not quit because filter is active)
        handle_key_event(&mut app, key(KeyCode::Esc));
        assert!(!app.has_filter());
        assert!(app.status_filter.is_none());
        assert!(!app.should_quit);
    }

    // =========================================================================
    // Delete/confirm mode tests
    // =========================================================================

    #[test]
    fn test_d_key_enters_confirm_mode() {
        let mut app = create_test_app_with_sessions(3);
        let selected_id = app.selected_session().map(|s| s.session_id.clone());
        handle_key_event(&mut app, key(KeyCode::Char('d')));

        assert!(matches!(app.mode, AppMode::Confirm { .. }));
        if let AppMode::Confirm { session_id, .. } = &app.mode {
            assert_eq!(Some(session_id.clone()), selected_id);
        }
    }

    #[test]
    fn test_d_key_no_op_when_no_sessions() {
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('d')));

        assert_eq!(app.mode, AppMode::Normal);
    }

    #[rstest]
    #[case::n_cancels(KeyCode::Char('n'))]
    #[case::esc_cancels(KeyCode::Esc)]
    fn test_confirm_cancel(#[case] cancel_key: KeyCode) {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('d')));
        assert!(matches!(app.mode, AppMode::Confirm { .. }));

        handle_key_event(&mut app, key(cancel_key));
        assert_eq!(app.mode, AppMode::Normal);
        // Sessions should remain unchanged
        assert_eq!(app.sessions.len(), 3);
    }

    #[test]
    fn test_confirm_ignores_unrelated_keys() {
        let mut app = create_test_app_with_sessions(3);
        handle_key_event(&mut app, key(KeyCode::Char('d')));
        assert!(matches!(app.mode, AppMode::Confirm { .. }));

        // Pressing 'j' or other keys should not change mode
        handle_key_event(&mut app, key(KeyCode::Char('j')));
        assert!(matches!(app.mode, AppMode::Confirm { .. }));
    }

    // =========================================================================
    // Two-stage delete tests (worktree cleanup prompt)
    //
    // These verify the state machine only. They do not exercise the actual
    // worktree cleanup path, which depends on external commands (tmux) and
    // is covered by integration tests on `cleanup_worktree_resources`.
    // =========================================================================

    use crate::shared::testing::TestRepo;

    fn session_with_cwd(id: &str, cwd: PathBuf) -> Session {
        Session {
            archived_at: None,
            pr_context: None,
            session_id: id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            is_delegated: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd,
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
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

    /// Creates a TestRepo with a single worktree named `feat` and returns it
    /// together with the resolved worktree path. Ownership of `TestRepo` is
    /// returned so the TempDir survives for the lifetime of the test.
    #[fixture]
    fn worktree_feat() -> (TestRepo, PathBuf) {
        let repo = TestRepo::new();
        repo.create_worktree("feat");
        let wt_path = repo.worktree_path("feat");
        (repo, wt_path)
    }

    #[rstest]
    fn test_confirm_delete_with_sibling_in_same_worktree_returns_to_normal(
        worktree_feat: (TestRepo, PathBuf),
    ) {
        // Two sessions share the same worktree. Deleting one must NOT trigger
        // worktree cleanup — the sibling session is still using it.
        let (_repo, wt_path) = worktree_feat;
        let s1 = session_with_cwd("s1", wt_path.join("src"));
        let s2 = session_with_cwd("s2", wt_path.clone());
        let mut app = App::with_sessions(vec![s1, s2]);

        handle_key_event(&mut app, key(KeyCode::Char('d')));
        assert!(matches!(app.mode, AppMode::Confirm { .. }));
        app.confirm_delete_with(|_| Ok(()))
            .expect("session deletion should succeed");

        assert_eq!(
            app.mode,
            AppMode::Normal,
            "sibling session exists; must not prompt for worktree cleanup"
        );
        // One session deleted, one remains
        assert_eq!(app.sessions.len(), 1);
    }

    #[rstest]
    fn test_request_delete_last_session_in_worktree_records_cleanup(
        worktree_feat: (TestRepo, PathBuf),
    ) {
        // Only session in the worktree. Confirm mode should carry the
        // worktree root so a single `y` deletes both.
        let (repo, wt_path) = worktree_feat;
        let s1 = session_with_cwd("s1", wt_path);
        let mut app = App::with_sessions(vec![s1]);

        handle_key_event(&mut app, key(KeyCode::Char('d')));

        match &app.mode {
            AppMode::Confirm {
                worktree_cleanup: Some(worktree_root),
                ..
            } => {
                assert!(
                    worktree_root.starts_with(repo.path()),
                    "worktree_root should be inside the main repo path"
                );
            }
            other => panic!("expected Confirm with worktree_cleanup=Some, got {other:?}"),
        }
    }

    #[rstest]
    fn test_confirm_delete_last_session_in_worktree_removes_worktree(
        worktree_feat: (TestRepo, PathBuf),
    ) {
        // Only session in the worktree. A single `y` must delete the
        // session AND clean up the worktree in one step.
        let (_repo, wt_path) = worktree_feat;
        let s1 = session_with_cwd("s1", wt_path.clone());
        let mut app = App::with_sessions(vec![s1]);

        handle_key_event(&mut app, key(KeyCode::Char('d')));
        app.confirm_delete_with(|_| Ok(()))
            .expect("session deletion should succeed");

        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.sessions.is_empty());
        assert!(
            !wt_path.exists(),
            "worktree directory should be removed after single confirmation"
        );
    }

    #[rstest]
    #[case::n_key(KeyCode::Char('n'))]
    #[case::esc_key(KeyCode::Esc)]
    fn test_confirm_delete_cancel_keeps_session_and_worktree(
        worktree_feat: (TestRepo, PathBuf),
        #[case] cancel_key: KeyCode,
    ) {
        let (_repo, wt_path) = worktree_feat;
        let s1 = session_with_cwd("s1", wt_path.clone());
        let mut app = App::with_sessions(vec![s1]);

        handle_key_event(&mut app, key(KeyCode::Char('d')));
        assert!(matches!(
            app.mode,
            AppMode::Confirm {
                worktree_cleanup: Some(_),
                ..
            }
        ));

        handle_key_event(&mut app, key(cancel_key));
        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.sessions.len(), 1);
        assert!(
            wt_path.exists(),
            "worktree directory should remain when confirmation is declined"
        );
    }

    // =========================================================================
    // Clean view key flow tests
    // =========================================================================

    use crate::commands::agent::tui::app::View;
    use crate::commands::agent::tui::clean_view::{CleanRow, CleanSection};

    fn clean_row(section: CleanSection, repo: &str, name: &str, path: &str) -> CleanRow {
        CleanRow {
            repo: repo.to_string(),
            branch: name.to_string(),
            name: name.to_string(),
            path: PathBuf::from(path),
            session_count: 0,
            has_active: false,
            updated_at: None,
            status_label: "PR merged".to_string(),
            pr_merged: section == CleanSection::ToDelete,
            section,
            sessions: Vec::new(),
        }
    }

    fn seed_one_worktree(app: &mut App) {
        app.set_worktrees(vec![crate::commands::agent::tui::worktree::WorktreeRow {
            repo: "r1".to_string(),
            branch: "feat-a".to_string(),
            name: "feat-a".to_string(),
            path: PathBuf::from("/tmp/r1/feat-a"),
            session_count: 0,
        }]);
    }

    #[test]
    fn pressing_c_from_session_view_enters_clean_and_requests_pr_fetch() {
        let mut app = create_test_app_with_sessions(1);
        seed_one_worktree(&mut app);
        let effects = handle_key_event(&mut app, key(KeyCode::Char('c')));
        assert_eq!(app.view, View::Clean);
        assert!(effects.request_clean_pr_fetch);
    }

    #[test]
    fn pressing_c_without_worktree_snapshot_defers_pr_fetch() {
        let mut app = create_test_app_with_sessions(0);
        let effects = handle_key_event(&mut app, key(KeyCode::Char('c')));
        assert_eq!(app.view, View::Clean);
        assert!(!effects.request_clean_pr_fetch);
        assert!(matches!(
            app.clean_view.state,
            crate::commands::agent::tui::clean_view::CleanLoadState::LoadingPr
        ));
    }

    #[test]
    fn seeding_after_empty_worktree_discovery_transitions_to_ready() {
        // Regression: discovery finishing with zero worktrees previously
        // left the view stuck on "Loading worktrees..." because the
        // empty-rows branch never transitioned out of LoadingPr.
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        app.set_worktrees(Vec::new());
        let seeded = app.seed_clean_view_if_pending();
        assert!(!seeded);
        assert!(matches!(
            app.clean_view.state,
            crate::commands::agent::tui::clean_view::CleanLoadState::Ready(_)
        ));
        assert_eq!(
            app.clean_view.pr_fetch,
            crate::commands::agent::tui::clean_view::PrFetchStatus::Done
        );
    }

    #[test]
    fn seeding_after_worktrees_arrive_kicks_off_pr_fetch() {
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        seed_one_worktree(&mut app);
        assert!(app.seed_clean_view_if_pending());
        assert!(matches!(
            app.clean_view.state,
            crate::commands::agent::tui::clean_view::CleanLoadState::Ready(_)
        ));
    }

    #[rstest]
    #[case::esc(KeyCode::Esc)]
    #[case::n(KeyCode::Char('n'))]
    #[case::q(KeyCode::Char('q'))]
    fn clean_view_cancel_returns_to_session_view(#[case] code: KeyCode) {
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        assert_eq!(app.view, View::Clean);
        handle_key_event(&mut app, key(code));
        assert_eq!(app.view, View::Session);
    }

    #[test]
    fn clean_view_enter_toggles_section() {
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        app.set_clean_rows(vec![
            clean_row(CleanSection::ToDelete, "r1", "feat-a", "/tmp/a"),
            clean_row(CleanSection::Kept, "r1", "fix-b", "/tmp/b"),
        ]);
        let before = app.clean_view.selected_row().expect("row");
        assert_eq!(before.section, CleanSection::ToDelete);

        handle_key_event(&mut app, key(KeyCode::Enter));

        let after = app.clean_view.selected_row().expect("row");
        assert_eq!(after.path, before.path);
        assert_eq!(after.section, CleanSection::Kept);
    }

    #[test]
    fn clean_view_y_with_pending_paths_emits_spawn_effect() {
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        app.set_clean_rows(vec![clean_row(
            CleanSection::ToDelete,
            "r1",
            "feat-a",
            "/tmp/a",
        )]);
        let effects = handle_key_event(&mut app, key(KeyCode::Char('y')));
        assert_eq!(app.view, View::Session); // returned to the session list
        assert_eq!(
            effects.spawn_detached_clean,
            Some(vec![PathBuf::from("/tmp/a")])
        );
    }

    #[test]
    fn clean_view_y_with_empty_to_delete_just_exits() {
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        app.set_clean_rows(vec![clean_row(CleanSection::Kept, "r1", "fix-b", "/tmp/b")]);
        let effects = handle_key_event(&mut app, key(KeyCode::Char('y')));
        assert_eq!(app.view, View::Session);
        assert!(effects.spawn_detached_clean.is_none());
    }

    #[test]
    fn tab_inside_clean_view_is_noop() {
        let mut app = create_test_app_with_sessions(0);
        handle_key_event(&mut app, key(KeyCode::Char('c')));
        let view_before = app.view;
        handle_key_event(&mut app, key(KeyCode::Tab));
        assert_eq!(app.view, view_before);
    }

    #[test]
    fn tab_switches_between_tasks_and_progress() {
        let mut app = create_test_app_with_sessions(1);

        handle_key_event(&mut app, key(KeyCode::Tab));
        let after_progress = app.progress_focused;
        handle_key_event(&mut app, key(KeyCode::Tab));
        let after_tasks = app.progress_focused;

        assert_eq!([after_progress, after_tasks], [true, false]);
    }

    #[test]
    fn test_confirm_delete_non_worktree_session_returns_to_normal() {
        // cwd is outside any git repository. No prompt, just plain delete.
        let tmp = tempfile::tempdir().expect("tempdir");
        let s1 = session_with_cwd("s1", tmp.path().to_path_buf());
        let mut app = App::with_sessions(vec![s1]);

        handle_key_event(&mut app, key(KeyCode::Char('d')));
        app.confirm_delete_with(|_| Ok(()))
            .expect("session deletion should succeed");

        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.sessions.is_empty());
    }
}
