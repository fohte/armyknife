use crate::commands::agent::store;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use super::clean_progress::{self, CleanLogEvent, TAIL_INTERVAL};
use super::clean_view::CleanRow;
use super::progress::{FetchedProgress, ProgressFetchScope};
use super::tq_snapshot::TqSnapshot;
use super::worktree::WorktreeRow;
use crate::commands::agent::types::Session;
use crate::infra::tq::TqClient;
use std::collections::HashSet;

/// Key event with code and modifiers.
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

/// Events that can occur in the TUI.
pub enum AppEvent {
    /// A key was pressed.
    Key(KeyEvent),
    /// The session database or one of its WAL files changed.
    SessionStoreChanged,
    /// Tick for periodic updates.
    Tick,
    /// Background worktree discovery finished.
    WorktreesLoaded(std::result::Result<Vec<super::worktree::WorktreeRow>, String>),
    /// Background session-label (repo name + worktree name) resolution
    /// finished for one batch of cwds.
    SessionLabelsResolved(Vec<(PathBuf, String, String)>),
    /// PR status fetch for the clean view completed.
    CleanPrFetched(std::result::Result<Vec<CleanRow>, String>),
    /// PR progress for the currently selected session finished loading.
    ProgressFetched {
        session_id: String,
        result: std::result::Result<FetchedProgress, String>,
    },
    /// One or more JSONL events from the detached clean child.
    CleanLogEvents(Vec<CleanLogEvent>),
    /// tq task snapshot fetch completed. `Ok(None)` means the fetch was
    /// skipped because tq is unavailable or there are no local sessions, so
    /// cached task links should remain in use.
    TqTasksFetched(std::result::Result<Option<TqSnapshot>, String>),
    /// `tq task url` fetch completed for a `t`-keypress request. `Ok(url)`
    /// is opened in the browser; `Err` (tq missing/unreachable) is logged
    /// and otherwise ignored -- the same silent degrade as every other tq
    /// integration point when `tq` can't run.
    TaskUrlFetched(std::result::Result<String, String>),
}

/// Event handler that combines keyboard input and file system events.
pub struct EventHandler {
    receiver: Receiver<AppEvent>,
    /// Cloned for ad-hoc background work (clean PR fetch, tail thread).
    sender: Sender<AppEvent>,
    /// Tokio runtime handle captured at construction so async PR fetches
    /// can be spawned without nesting a fresh runtime — `agent watch` is
    /// invoked from inside the top-level tokio runtime.
    rt_handle: Option<tokio::runtime::Handle>,
    /// Watcher must be kept alive to receive events.
    _watcher: Option<RecommendedWatcher>,
}

impl EventHandler {
    /// Creates a new event handler.
    /// Spawns background threads for keyboard and file system monitoring.
    pub fn new() -> Result<Self> {
        let (tx, rx) = mpsc::channel();

        // Spawn keyboard event thread
        let key_tx = tx.clone();
        thread::spawn(move || {
            handle_keyboard_events(key_tx);
        });

        // Spawn tick thread for periodic updates
        let tick_tx = tx.clone();
        thread::spawn(move || {
            handle_tick_events(tick_tx);
        });

        // Spawn worktree discovery thread (one-shot).
        let wt_tx = tx.clone();
        thread::spawn(move || {
            handle_worktree_discovery(wt_tx);
        });

        // Run stale cleanup off the startup path.
        let cleanup_tx = tx.clone();
        thread::spawn(move || {
            handle_stale_session_cleanup(cleanup_tx);
        });

        // Set up file system watcher
        let sender = tx.clone();
        let watcher = setup_file_watcher(tx)?;

        Ok(Self {
            receiver: rx,
            sender,
            rt_handle: tokio::runtime::Handle::try_current().ok(),
            _watcher: watcher,
        })
    }

    /// Receives the next event, blocking until one is available.
    pub fn next(&self) -> Result<AppEvent> {
        self.receiver
            .recv()
            .map_err(|e| anyhow::anyhow!("Event channel closed: {}", e))
    }

    /// Non-blocking receive: returns `Some(event)` if available, `None` otherwise.
    pub fn try_next(&self) -> Option<AppEvent> {
        self.receiver.try_recv().ok()
    }

    /// Resolve session labels (repo + worktree names) for `cwds` in the
    /// background. The result arrives as [`AppEvent::SessionLabelsResolved`].
    /// Callers must deduplicate cwds (see `App::claim_unresolved_label_cwds`).
    pub fn start_session_labels_resolve(&self, cwds: Vec<PathBuf>) {
        if cwds.is_empty() {
            return;
        }
        let tx = self.sender.clone();
        thread::spawn(move || {
            let results = super::app::resolve_labels_for_cwds(&cwds);
            let _ = tx.send(AppEvent::SessionLabelsResolved(results));
        });
    }

    /// Kick off a one-shot PR-status fetch for the clean view. Returns
    /// immediately; the result arrives as [`AppEvent::CleanPrFetched`].
    /// If the runtime handle is unavailable, the failure is reported
    /// through the same event so the user always gets a banner.
    pub fn start_clean_pr_fetch(&self, rows: Vec<WorktreeRow>, sessions: Vec<Session>) {
        let tx = self.sender.clone();
        let Some(rt) = self.rt_handle.as_ref().cloned() else {
            let _ = tx.send(AppEvent::CleanPrFetched(Err(
                "tokio runtime is not available".to_string(),
            )));
            return;
        };
        rt.spawn(async move {
            let result = super::pr_fetch::fetch_clean_inputs(rows, sessions).await;
            let _ = tx.send(AppEvent::CleanPrFetched(result));
        });
    }

    /// Fetch delegated sessions and their PR progress for the selected task.
    pub fn start_progress_fetch(&self, session_id: String, fetch_scope: ProgressFetchScope) {
        let tx = self.sender.clone();
        let Some(rt) = self.rt_handle.as_ref().cloned() else {
            let _ = tx.send(AppEvent::ProgressFetched {
                session_id,
                result: Err("tokio runtime is not available".to_string()),
            });
            return;
        };
        rt.spawn(async move {
            let result =
                super::progress_fetch::fetch_progress(session_id.clone(), fetch_scope).await;
            let _ = tx.send(AppEvent::ProgressFetched { session_id, result });
        });
    }

    /// Kick off a one-shot fetch of the tq task links. Returns
    /// immediately; the result arrives as [`AppEvent::TqTasksFetched`].
    /// Same runtime-unavailable
    /// fallback as [`Self::start_clean_pr_fetch`].
    pub fn start_tq_task_fetch(&self, local_session_ids: HashSet<String>) {
        let tx = self.sender.clone();
        let Some(rt) = self.rt_handle.as_ref().cloned() else {
            let _ = tx.send(AppEvent::TqTasksFetched(Err(
                "tokio runtime is not available".to_string(),
            )));
            return;
        };
        let client = TqClient::detect();
        rt.spawn(async move {
            let result = super::tq_fetch::fetch_task_snapshot(client, local_session_ids).await;
            let _ = tx.send(AppEvent::TqTasksFetched(result));
        });
    }

    /// Kick off a one-shot fetch of `task_id`'s web URL via `tq`. Returns
    /// immediately; the result arrives as [`AppEvent::TaskUrlFetched`].
    /// Same runtime-unavailable fallback as [`Self::start_clean_pr_fetch`].
    pub fn start_task_url_fetch(&self, task_id: String) {
        let tx = self.sender.clone();
        let Some(rt) = self.rt_handle.as_ref().cloned() else {
            let _ = tx.send(AppEvent::TaskUrlFetched(Err(
                "tokio runtime is not available".to_string(),
            )));
            return;
        };
        let client = TqClient::detect();
        rt.spawn(async move {
            let result = match client {
                Some(client) => client.task_url(&task_id).await.map_err(|e| e.to_string()),
                None => Err("tq is not on PATH".to_string()),
            };
            let _ = tx.send(AppEvent::TaskUrlFetched(result));
        });
    }

    /// Begin tailing `log_path` for JSONL events from the detached
    /// clean child. Stops on the first `Done` event or when the
    /// receiver is dropped. Polling cadence matches
    /// [`clean_progress::TAIL_INTERVAL`].
    pub fn start_clean_tail(&self, initial_log_path: PathBuf, run_id: String) {
        let tx = self.sender.clone();
        thread::spawn(move || {
            let mut log_path = initial_log_path;
            let mut cursor = 0u64;
            // The pre-`Start` phase is bounded by polling count so we
            // do not loop forever when the child dies before writing
            // anything. Past `Start`, a single large `rm -rf` may write
            // no events for minutes — the post-`Start` exit condition
            // is simply receiving the matching `Done` event.
            let mut started = false;
            let mut empty_polls = 0usize;
            loop {
                thread::sleep(TAIL_INTERVAL);
                // Date rollover: today's log path changes at midnight UTC.
                // When that happens, the new file starts at offset 0 so
                // we reset the cursor before reading.
                if let Some(today) = clean_progress::live_log_path()
                    && today != log_path
                {
                    log_path = today;
                    cursor = 0;
                }
                let (events, new_cursor) =
                    match clean_progress::read_new_events(&log_path, cursor, &run_id) {
                        Ok(pair) => pair,
                        Err(_) => break,
                    };
                cursor = new_cursor;
                let done = events
                    .iter()
                    .any(|e| matches!(e, CleanLogEvent::Done { .. }));
                if !events.is_empty() {
                    if events
                        .iter()
                        .any(|e| matches!(e, CleanLogEvent::Start { .. }))
                    {
                        started = true;
                    }
                    empty_polls = 0;
                    if tx.send(AppEvent::CleanLogEvents(events)).is_err() {
                        break;
                    }
                    if done {
                        break;
                    }
                } else if !started {
                    empty_polls += 1;
                    if empty_polls > 120 {
                        break;
                    }
                }
            }
        });
    }
}

/// Handles keyboard events in a background thread.
fn handle_keyboard_events(tx: Sender<AppEvent>) {
    loop {
        // Poll for events with a short timeout
        if event::poll(Duration::from_millis(50)).unwrap_or(false)
            && let Ok(Event::Key(key)) = event::read()
            && key.kind == KeyEventKind::Press
        {
            let key_event = KeyEvent {
                code: key.code,
                modifiers: key.modifiers,
            };
            if tx.send(AppEvent::Key(key_event)).is_err() {
                // Channel closed, exit thread
                break;
            }
        }
    }
}

/// Performs a one-shot scan for worktrees under the configured repos root.
/// Sends a single `WorktreesLoaded` event when finished.
fn handle_worktree_discovery(tx: Sender<AppEvent>) {
    use crate::shared::config::load_config;
    use crate::shared::repos_root::resolve_repos_root;

    let result = (|| -> std::result::Result<Vec<super::worktree::WorktreeRow>, String> {
        let config = load_config().map_err(|e| e.to_string())?;
        let repos_root = resolve_repos_root(config.agent.worktree.repos_root.as_deref())
            .map_err(|e| e.to_string())?;
        Ok(super::worktree::discover_worktree_rows(
            &repos_root,
            &config.agent.worktree.dir,
        ))
    })();

    let _ = tx.send(AppEvent::WorktreesLoaded(result));
}

/// One-shot stale-session cleanup run from `EventHandler::new`.
fn handle_stale_session_cleanup(tx: Sender<AppEvent>) {
    if let Ok(true) = store::cleanup_stale_sessions() {
        let _ = tx.send(AppEvent::SessionStoreChanged);
    }
}

/// Handles tick events for periodic UI updates.
fn handle_tick_events(tx: Sender<AppEvent>) {
    loop {
        thread::sleep(Duration::from_secs(1));
        if tx.send(AppEvent::Tick).is_err() {
            // Channel closed, exit thread
            break;
        }
    }
}

/// Sets up a file system watcher for the session database and its WAL files.
fn setup_file_watcher(tx: Sender<AppEvent>) -> Result<Option<RecommendedWatcher>> {
    let sessions_dir = match store::sessions_dir() {
        Ok(dir) => dir,
        Err(_) => return Ok(None),
    };

    // Create sessions directory if it doesn't exist
    if !sessions_dir.exists() {
        std::fs::create_dir_all(&sessions_dir)?;
    }

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res
            && extract_session_store_change(&event)
        {
            let _ = tx.send(AppEvent::SessionStoreChanged);
        }
    })?;

    watcher.watch(Path::new(&sessions_dir), RecursiveMode::NonRecursive)?;

    Ok(Some(watcher))
}

/// Returns whether a filesystem event may reflect a session database update.
fn extract_session_store_change(event: &notify::Event) -> bool {
    let is_database_file = |path: &Path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "sessions.db" || name == "sessions.db-wal")
    };

    match event.kind {
        notify::EventKind::Modify(
            notify::event::ModifyKind::Data(_) | notify::event::ModifyKind::Any,
        ) => event.paths.iter().any(|path| is_database_file(path)),
        notify::EventKind::Create(
            notify::event::CreateKind::File | notify::event::CreateKind::Any,
        ) => event
            .paths
            .iter()
            .any(|path| path.file_name().and_then(|name| name.to_str()) == Some("sessions.db")),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::path::PathBuf;

    #[test]
    fn test_app_event_enum() {
        // Just verify the enum variants can be created
        let _key = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
        });
        let _store_changed = AppEvent::SessionStoreChanged;
        let _tick = AppEvent::Tick;
    }

    #[rstest]
    #[case::database_data(
        "/sessions/sessions.db",
        notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any
        )),
        true
    )]
    #[case::wal_data(
        "/sessions/sessions.db-wal",
        notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any
        )),
        true
    )]
    #[case::database_create(
        "/sessions/sessions.db",
        notify::EventKind::Create(notify::event::CreateKind::File),
        true
    )]
    #[case::wal_create_ignored(
        "/sessions/sessions.db-wal",
        notify::EventKind::Create(notify::event::CreateKind::File),
        false
    )]
    #[case::wal_remove_ignored(
        "/sessions/sessions.db-wal",
        notify::EventKind::Remove(notify::event::RemoveKind::File),
        false
    )]
    #[case::database_remove_ignored(
        "/sessions/sessions.db",
        notify::EventKind::Remove(notify::event::RemoveKind::File),
        false
    )]
    #[case::shared_memory_ignored(
        "/sessions/sessions.db-shm",
        notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any
        )),
        false
    )]
    #[case::legacy_json_ignored(
        "/sessions/old-session.json",
        notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any
        )),
        false
    )]
    #[case::unrelated_file_ignored(
        "/sessions/notes.txt",
        notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any
        )),
        false
    )]
    fn test_extract_session_store_change(
        #[case] path: &str,
        #[case] kind: notify::EventKind,
        #[case] expected: bool,
    ) {
        let event = notify::Event {
            kind,
            paths: vec![PathBuf::from(path)],
            attrs: Default::default(),
        };

        assert_eq!(extract_session_store_change(&event), expected);
    }

    #[test]
    fn test_try_next_returns_none_on_empty_channel() {
        let (tx, rx) = mpsc::channel::<AppEvent>();
        let handler = EventHandler {
            receiver: rx,
            sender: tx,
            rt_handle: None,
            _watcher: None,
        };
        assert!(handler.try_next().is_none());
    }

    #[test]
    fn test_try_next_returns_events_then_none() {
        let (tx, rx) = mpsc::channel::<AppEvent>();
        let handler = EventHandler {
            receiver: rx,
            sender: tx.clone(),
            rt_handle: None,
            _watcher: None,
        };

        tx.send(AppEvent::Tick).unwrap();
        tx.send(AppEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::NONE,
        }))
        .unwrap();

        // First call returns the Tick
        let first = handler.try_next();
        assert!(first.is_some());

        // Second call returns the Key
        let second = handler.try_next();
        assert!(second.is_some());

        // Third call returns None (queue drained)
        assert!(handler.try_next().is_none());
    }

    #[test]
    fn test_try_next_drains_all_queued_events() {
        let (tx, rx) = mpsc::channel::<AppEvent>();
        let handler = EventHandler {
            receiver: rx,
            sender: tx.clone(),
            rt_handle: None,
            _watcher: None,
        };

        // Enqueue multiple session-store changes.
        for _ in 0..5 {
            tx.send(AppEvent::SessionStoreChanged).unwrap();
        }

        let mut count = 0;
        while handler.try_next().is_some() {
            count += 1;
        }
        assert_eq!(count, 5);
    }

    #[test]
    fn test_extract_session_store_change_accepts_database_paths_in_batch_events() {
        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![
                PathBuf::from("/sessions/old-session.json"),
                PathBuf::from("/sessions/sessions.db-wal"),
            ],
            attrs: Default::default(),
        };

        assert!(extract_session_store_change(&event));
    }
}
