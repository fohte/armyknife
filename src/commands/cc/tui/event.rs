use crate::commands::cc::store;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use notify::{
    EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{CreateKind, ModifyKind, RemoveKind},
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use super::clean_progress::{self, CleanLogEvent, TAIL_INTERVAL};
use super::clean_view::CleanRow;
use super::worktree_view::WorktreeRow;
use crate::commands::cc::types::Session;

/// Key event with code and modifiers.
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

/// Represents a change to a specific session file.
#[derive(Debug, Clone)]
pub struct SessionChange {
    pub session_id: String,
    pub change_type: SessionChangeType,
}

/// Type of change that occurred to a session file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionChangeType {
    Created,
    Modified,
    Deleted,
}

/// Events that can occur in the TUI.
pub enum AppEvent {
    /// A key was pressed.
    Key(KeyEvent),
    /// Session data changed on disk.
    /// Contains session changes if specific files were identified, or None for full reload.
    SessionsChanged(Option<Vec<SessionChange>>),
    /// Tick for periodic updates.
    Tick,
    /// Background worktree discovery finished.
    WorktreesLoaded(std::result::Result<Vec<super::worktree_view::WorktreeRow>, String>),
    /// PR status fetch for the clean view completed.
    CleanPrFetched(std::result::Result<Vec<CleanRow>, String>),
    /// One or more JSONL events from the detached clean child.
    CleanLogEvents(Vec<CleanLogEvent>),
}

/// Event handler that combines keyboard input and file system events.
pub struct EventHandler {
    receiver: Receiver<AppEvent>,
    /// Cloned for ad-hoc background work (clean PR fetch, tail thread).
    sender: Sender<AppEvent>,
    /// Tokio runtime handle captured at construction so async PR fetches
    /// can be spawned without nesting a fresh runtime — `cc watch` is
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

    let result = (|| -> std::result::Result<Vec<super::worktree_view::WorktreeRow>, String> {
        let config = load_config().map_err(|e| e.to_string())?;
        let repos_root =
            resolve_repos_root(config.wm.repos_root.as_deref()).map_err(|e| e.to_string())?;
        Ok(super::worktree_view::discover_worktree_rows(
            &repos_root,
            &config.wm.worktrees_dir,
        ))
    })();

    let _ = tx.send(AppEvent::WorktreesLoaded(result));
}

/// One-shot stale-session cleanup, run off the startup path.
/// Triggers a full reload only when something was actually removed,
/// so the common no-op case does not pay another `list_sessions` scan.
fn handle_stale_session_cleanup(tx: Sender<AppEvent>) {
    if let Ok(true) = store::cleanup_stale_sessions() {
        let _ = tx.send(AppEvent::SessionsChanged(None));
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

/// Sets up a file system watcher for the sessions directory.
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
        if let Ok(event) = res {
            let changes = extract_session_changes(&event);
            if !changes.is_empty() {
                let _ = tx.send(AppEvent::SessionsChanged(Some(changes)));
            }
        }
    })?;

    watcher.watch(Path::new(&sessions_dir), RecursiveMode::NonRecursive)?;

    Ok(Some(watcher))
}

/// Extracts session IDs from file paths in the event.
fn extract_session_changes(event: &notify::Event) -> Vec<SessionChange> {
    let change_type = match event.kind {
        EventKind::Create(CreateKind::File) => SessionChangeType::Created,
        EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Name(_)) => SessionChangeType::Modified,
        EventKind::Remove(RemoveKind::File) => SessionChangeType::Deleted,
        _ => return Vec::new(),
    };

    event
        .paths
        .iter()
        .filter_map(|path| {
            // Only process .json files (not .lock or .tmp)
            if path.extension()?.to_str()? != "json" {
                return None;
            }
            let session_id = path.file_stem()?.to_str()?.to_string();
            Some(SessionChange {
                session_id,
                change_type,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{DataChange, ModifyKind};
    use rstest::rstest;
    use std::path::PathBuf;

    #[test]
    fn test_app_event_enum() {
        // Just verify the enum variants can be created
        let _key = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
        });
        let _changed = AppEvent::SessionsChanged(None);
        let _tick = AppEvent::Tick;
    }

    #[rstest]
    #[case::json_file(
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        "/sessions/test-123.json",
        Some(("test-123", SessionChangeType::Modified))
    )]
    #[case::lock_file_ignored(
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        "/sessions/test-123.json.lock",
        None
    )]
    #[case::tmp_file_ignored(
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        "/sessions/test-123.json.tmp",
        None
    )]
    #[case::create_event(
        EventKind::Create(CreateKind::File),
        "/sessions/new-session.json",
        Some(("new-session", SessionChangeType::Created))
    )]
    #[case::delete_event(
        EventKind::Remove(RemoveKind::File),
        "/sessions/deleted-session.json",
        Some(("deleted-session", SessionChangeType::Deleted))
    )]
    fn test_extract_session_changes(
        #[case] kind: EventKind,
        #[case] path: &str,
        #[case] expected: Option<(&str, SessionChangeType)>,
    ) {
        let event = notify::Event {
            kind,
            paths: vec![PathBuf::from(path)],
            attrs: Default::default(),
        };

        let changes = extract_session_changes(&event);

        match expected {
            Some((session_id, change_type)) => {
                assert_eq!(changes.len(), 1);
                assert_eq!(changes[0].session_id, session_id);
                assert_eq!(changes[0].change_type, change_type);
            }
            None => {
                assert!(changes.is_empty());
            }
        }
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

        // Enqueue multiple SessionsChanged events
        for i in 0..5 {
            tx.send(AppEvent::SessionsChanged(Some(vec![SessionChange {
                session_id: format!("session-{i}"),
                change_type: SessionChangeType::Modified,
            }])))
            .unwrap();
        }

        let mut count = 0;
        while handler.try_next().is_some() {
            count += 1;
        }
        assert_eq!(count, 5);
    }

    #[test]
    fn test_extract_session_changes_multiple_files() {
        let event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![
                PathBuf::from("/sessions/session-1.json"),
                PathBuf::from("/sessions/session-2.json"),
                PathBuf::from("/sessions/session-3.json.lock"), // should be ignored
            ],
            attrs: Default::default(),
        };

        let changes = extract_session_changes(&event);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].session_id, "session-1");
        assert_eq!(changes[1].session_id, "session-2");
    }
}
