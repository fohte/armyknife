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

    pub(super) fn subscribe(&self) -> broadcast::Receiver<()> {
        self.events.subscribe()
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
    fn forwards_database_change_events_to_subscribers(
        events_channel: (broadcast::Sender<()>, broadcast::Receiver<()>),
    ) {
        let (event_sender, mut event_receiver) = events_channel;
        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: vec![PathBuf::from("/fixture-store/sessions.db")],
            attrs: Default::default(),
        };

        forward_session_store_event(Ok(event), &event_sender);
        let received = event_receiver.try_recv().is_ok();
        let expected = true;

        assert_eq!(received, expected);
    }

    #[rstest]
    fn forwards_watcher_errors_as_refresh_signals(
        events_channel: (broadcast::Sender<()>, broadcast::Receiver<()>),
    ) {
        let (event_sender, mut event_receiver) = events_channel;

        forward_session_store_event(
            Err(notify::Error::generic("fixture watcher error")),
            &event_sender,
        );
        let received = event_receiver.try_recv().is_ok();
        let expected = true;

        assert_eq!(received, expected);
    }
}
