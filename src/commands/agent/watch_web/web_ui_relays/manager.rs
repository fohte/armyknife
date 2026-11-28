use std::future::pending;
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;

use super::{Relay, RelayKey, WebUiRelays, relay_url, web_ui_proxy};
use crate::commands::agent::types::WebUiRegistration;

// Coalesce the database and WAL notifications emitted by one store update.
const STORE_EVENT_DEBOUNCE: Duration = Duration::from_millis(150);

impl WebUiRelays {
    pub(super) async fn start_relay(
        &self,
        key: &RelayKey,
        registration: &WebUiRegistration,
    ) -> Result<Relay> {
        let upstream_port = super::super::super::web_ui::local_web_ui_port(&registration.url)?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        let url = relay_url(&registration.url, port)?;
        let client = web_ui_proxy::new_client()?;
        let session_id = key.session_id.clone();
        let web_ui_name = key.name.clone();
        let relay_server = tokio::spawn(async move {
            if let Err(error) =
                axum::serve(listener, web_ui_proxy::router(upstream_port, client)).await
            {
                tracing::warn!(
                    session = %session_id,
                    web_ui = %web_ui_name,
                    %error,
                    "web UI relay server stopped"
                );
            }
        });
        let server_abort = relay_server.abort_handle();
        let relay_exit_sender = self.relay_exit_sender.clone();
        let relay_key = key.clone();
        let server = tokio::spawn(async move {
            if let Err(error) = relay_server.await
                && !error.is_cancelled()
            {
                tracing::warn!(
                    session = %relay_key.session_id,
                    web_ui = %relay_key.name,
                    %error,
                    "web UI relay task failed"
                );
            }
            if let Some(sender) = relay_exit_sender {
                let _ = sender.send(relay_key);
            }
        });
        #[cfg(test)]
        self.relay_abort_handles
            .lock()
            .expect("relay abort handle lock should succeed")
            .push(server_abort.clone());

        Ok(Relay {
            url,
            port,
            exposed_url: None,
            exposed: false,
            registration_saved: false,
            server_abort,
            server,
        })
    }

    pub(in crate::commands::agent::watch_web) async fn start(
        self,
        shutdown: watch::Receiver<bool>,
        store_events: broadcast::Receiver<()>,
    ) -> JoinHandle<()> {
        tokio::spawn(self.run_event_driven(shutdown, store_events))
    }

    async fn run_event_driven(
        mut self,
        mut shutdown: watch::Receiver<bool>,
        mut store_events: broadcast::Receiver<()>,
    ) {
        let (relay_exit_sender, mut relay_exits) = mpsc::unbounded_channel();
        self.relay_exit_sender = Some(relay_exit_sender);
        let mut store_events_open = true;
        if !*shutdown.borrow() {
            self.sync_and_record_failure("at startup").await;
        }
        loop {
            let retry_at = self.next_attempt_at(Instant::now());
            tokio::select! {
                event = store_events.recv(), if store_events_open => {
                    match event {
                        Err(broadcast::error::RecvError::Closed) => store_events_open = false,
                        Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                            tokio::time::sleep(STORE_EVENT_DEBOUNCE).await;
                            loop {
                                match store_events.try_recv() {
                                    Ok(()) | Err(broadcast::error::TryRecvError::Lagged(_)) => {}
                                    Err(broadcast::error::TryRecvError::Closed) => {
                                        store_events_open = false;
                                        break;
                                    }
                                    Err(broadcast::error::TryRecvError::Empty) => break,
                                }
                            }
                            self.sync_and_record_failure("after a session store change")
                                .await;
                        }
                    }
                }
                relay_exit = relay_exits.recv() => {
                    if relay_exit.is_some() {
                        self.sync_and_record_failure("after a relay exit").await;
                    }
                }
                _ = async {
                    if let Some(retry_at) = retry_at {
                        tokio::time::sleep_until(retry_at.into()).await;
                    } else {
                        pending::<()>().await;
                    }
                } => self.sync_and_record_failure("after a retry deadline").await,
                result = shutdown.changed() => {
                    if result.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
        }

        self.stop_all().await;
    }

    async fn sync_and_record_failure(&mut self, timing: &str) {
        match self.sync().await {
            Ok(()) => self.expose_backoff.clear_sync_failure(),
            Err(error) => {
                self.expose_backoff.record_sync_failure(Instant::now());
                #[cfg(test)]
                if let Some(sender) = &self.sync_error_sender {
                    let _ = sender.send(());
                }
                tracing::warn!(%error, timing, "failed to synchronize web UI relays");
            }
        }
    }

    fn next_attempt_at(&mut self, now: Instant) -> Option<Instant> {
        [
            self.expose_backoff.next_attempt_at(now),
            self.unexpose_backoff.next_attempt_at(now),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}
