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
                forward_session_store_event(event, &event_sender);
            })?;
        watcher.watch(sessions_dir, RecursiveMode::NonRecursive)?;

        Ok(Self {
            _watcher: watcher,
            events,
        })
    }

    pub(super) fn start_or_warn(sessions_dir: &Path) -> Option<Self> {
        Self::start_or_warn_with(sessions_dir, Self::start)
    }

    fn start_or_warn_with(
        sessions_dir: &Path,
        start: impl FnOnce(&Path) -> notify::Result<Self>,
    ) -> Option<Self> {
        match start(sessions_dir) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "failed to start session store watcher; revisions will refresh on connection and web UI relays will sync only at startup"
                );
                None
            }
        }
    }

    pub(super) fn subscribe(&self) -> broadcast::Receiver<()> {
        self.events.subscribe()
    }

    pub(super) fn subscribe_or_closed(watcher: Option<&Self>) -> broadcast::Receiver<()> {
        watcher
            .map(Self::subscribe)
            .unwrap_or_else(Self::closed_receiver)
    }

    fn closed_receiver() -> broadcast::Receiver<()> {
        let (sender, receiver) = broadcast::channel(1);
        drop(sender);
        receiver
    }
}

fn forward_session_store_event(
    event: notify::Result<notify::Event>,
    event_sender: &broadcast::Sender<()>,
) {
    match event {
        Ok(event) if is_session_store_change(&event) => {
            let _ = event_sender.send(());
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(%error, "session store filesystem watcher reported an error");
            let _ = event_sender.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use std::path::PathBuf;

    #[fixture]
    fn events_channel() -> (broadcast::Sender<()>, broadcast::Receiver<()>) {
        broadcast::channel(1)
    }

    #[rstest]
    #[case::database_change(Ok(notify::Event {
        kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any,
        )),
        paths: vec![PathBuf::from("/fixture-store/sessions.db")],
        attrs: Default::default(),
    }))]
    #[case::watcher_error(Err(notify::Error::generic("fixture watcher error")))]
    fn forwards_store_changes_and_watcher_errors_as_refresh_signals(
        events_channel: (broadcast::Sender<()>, broadcast::Receiver<()>),
        #[case] event: notify::Result<notify::Event>,
    ) {
        let (event_sender, mut event_receiver) = events_channel;

        forward_session_store_event(event, &event_sender);
        let received = event_receiver.try_recv().is_ok();
        let expected = true;

        assert_eq!(received, expected);
    }

    #[test]
    fn keeps_watcher_unavailable_when_a_missing_sessions_dir_cannot_be_watched() {
        let temp_dir = tempfile::tempdir().expect("temporary directory should be created");
        let sessions_dir = temp_dir.path().join("sessions");
        let started_before_directory_existed = std::cell::Cell::new(false);

        let watcher = SessionStoreWatcher::start_or_warn_with(&sessions_dir, |path| {
            started_before_directory_existed.set(!path.exists());
            Err(notify::Error::generic("fixture watcher error"))
        });

        assert_eq!(
            (
                watcher.is_none(),
                started_before_directory_existed.get(),
                sessions_dir.exists(),
            ),
            (true, true, false),
        );
    }
}
