use std::path::Path;

use notify::{RecursiveMode, Watcher};
use tokio::sync::broadcast;

use crate::commands::agent::session_store_event::is_session_store_change;

pub(super) struct SessionStoreWatcher {
    _watcher: notify::RecommendedWatcher,
    events: broadcast::Sender<()>,
}

impl SessionStoreWatcher {
    pub(super) fn start(sessions_dir: &Path) -> notify::Result<Self> {
        let (events, _) = broadcast::channel(64);
        let event_sender = events.clone();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.is_ok_and(|event| is_session_store_change(&event)) {
                    let _ = event_sender.send(());
                }
            })?;
        watcher.watch(sessions_dir, RecursiveMode::NonRecursive)?;

        Ok(Self {
            _watcher: watcher,
            events,
        })
    }

    pub(super) fn subscribe(&self) -> broadcast::Receiver<()> {
        self.events.subscribe()
    }
}
