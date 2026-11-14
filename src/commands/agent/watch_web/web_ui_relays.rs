use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Url;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::web_ui_proxy;
use crate::commands::agent::store;
use crate::commands::agent::types::{SessionStatus, WebUiRegistration};
use crate::shared::hooks;

const SYNC_INTERVAL: Duration = Duration::from_secs(1);
const WEB_UI_URL_ENV: &str = "ARMYKNIFE_WEB_UI_URL";
const WEB_UI_PORT_ENV: &str = "ARMYKNIFE_WEB_UI_PORT";
const WEB_UI_EXPOSED_URL_ENV: &str = "ARMYKNIFE_WEB_UI_EXPOSED_URL";

type HookRunner = dyn Fn(&str, &[(&str, &str)]) -> Result<Option<String>> + Send + Sync;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RelayKey {
    session_id: String,
    name: String,
    url: String,
}

impl RelayKey {
    fn new(session_id: &str, registration: &WebUiRegistration) -> Self {
        Self {
            session_id: session_id.to_owned(),
            name: registration.name.clone(),
            url: registration.url.clone(),
        }
    }
}

struct Relay {
    url: String,
    port: u16,
    exposed_url: Option<String>,
    server: JoinHandle<()>,
}

pub(super) struct WebUiRelays {
    sessions_dir: PathBuf,
    hook_runner: Arc<HookRunner>,
    active: HashMap<RelayKey, Relay>,
}

impl WebUiRelays {
    pub(super) fn new(sessions_dir: PathBuf) -> Self {
        Self::with_hook_runner(sessions_dir, hooks::run_hook_capture_stdout)
    }

    fn with_hook_runner(
        sessions_dir: PathBuf,
        hook_runner: impl Fn(&str, &[(&str, &str)]) -> Result<Option<String>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            sessions_dir,
            hook_runner: Arc::new(hook_runner),
            active: HashMap::new(),
        }
    }

    pub(super) async fn start(mut self, shutdown: watch::Receiver<bool>) -> JoinHandle<()> {
        if let Err(error) = self.sync().await {
            tracing::warn!(%error, "failed to synchronize web UI relays at startup");
        }

        tokio::spawn(self.run_periodic(shutdown))
    }

    async fn run_periodic(mut self, mut shutdown: watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(SYNC_INTERVAL);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(error) = self.sync().await {
                        tracing::warn!(%error, "failed to synchronize web UI relays");
                    }
                }
                result = shutdown.changed() => {
                    if result.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
        }

        self.stop_all().await;
    }

    async fn sync(&mut self) -> Result<()> {
        let sessions_dir = self.sessions_dir.clone();
        let sessions =
            tokio::task::spawn_blocking(move || store::list_all_sessions_from(&sessions_dir))
                .await
                .context("failed to join web UI session lookup")??;
        let desired = sessions
            .iter()
            .filter(|session| session.status != SessionStatus::Ended)
            .flat_map(|session| {
                session.web_uis.iter().map(move |registration| {
                    (
                        RelayKey::new(&session.session_id, registration),
                        registration.clone(),
                    )
                })
            })
            .collect::<HashMap<_, _>>();
        let desired_keys = desired.keys().cloned().collect::<HashSet<_>>();
        let stale_keys = self
            .active
            .keys()
            .filter(|key| !desired_keys.contains(*key))
            .cloned()
            .collect::<Vec<_>>();

        for key in stale_keys {
            if let Some(relay) = self.active.remove(&key) {
                self.stop_relay(&key, relay, true).await;
            }
        }

        for (key, registration) in desired {
            if self.active.contains_key(&key) {
                continue;
            }
            match self.start_relay(&key, &registration).await {
                Ok(relay) => {
                    let exposed_url = relay.exposed_url.clone();
                    self.active.insert(key.clone(), relay);
                    if let Err(error) =
                        persist_exposed_url(self.sessions_dir.clone(), key.clone(), exposed_url)
                            .await
                    {
                        tracing::warn!(
                            session = %key.session_id,
                            web_ui = %key.name,
                            %error,
                            "failed to save exposed web UI URL"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        session = %key.session_id,
                        web_ui = %key.name,
                        %error,
                        "failed to start web UI relay"
                    );
                }
            }
        }
        Ok(())
    }

    async fn start_relay(&self, key: &RelayKey, registration: &WebUiRegistration) -> Result<Relay> {
        let upstream_port = super::super::web_ui::local_web_ui_port(&registration.url)?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        let url = relay_url(&registration.url, port)?;
        let server = tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                web_ui_proxy::router(upstream_port, reqwest::Client::new()),
            )
            .await;
        });

        let hook_url = url.clone();
        let hook_port = port.to_string();
        let exposed_url = invoke_hook(
            self.hook_runner.clone(),
            "web-ui-expose",
            vec![
                (WEB_UI_URL_ENV.to_owned(), hook_url),
                (WEB_UI_PORT_ENV.to_owned(), hook_port),
            ],
        )
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(
                session = %key.session_id,
                web_ui = %key.name,
                %error,
                "web-ui-expose hook failed"
            );
            None
        })
        .and_then(valid_exposed_url);

        Ok(Relay {
            url,
            port,
            exposed_url,
            server,
        })
    }

    async fn stop_all(&mut self) {
        let relays = self.active.drain().collect::<Vec<_>>();
        for (key, relay) in relays {
            self.stop_relay(&key, relay, true).await;
        }
    }

    async fn stop_relay(&self, key: &RelayKey, relay: Relay, clear_exposed_url: bool) {
        let exposed_url = relay.exposed_url.as_deref().unwrap_or_default().to_owned();
        let port = relay.port.to_string();
        if let Err(error) = invoke_hook(
            self.hook_runner.clone(),
            "web-ui-unexpose",
            vec![
                (WEB_UI_URL_ENV.to_owned(), relay.url.clone()),
                (WEB_UI_PORT_ENV.to_owned(), port),
                (WEB_UI_EXPOSED_URL_ENV.to_owned(), exposed_url),
            ],
        )
        .await
        {
            tracing::warn!(
                session = %key.session_id,
                web_ui = %key.name,
                %error,
                "web-ui-unexpose hook failed"
            );
        }
        relay.server.abort();
        if let Err(error) = relay.server.await
            && !error.is_cancelled()
        {
            tracing::warn!(
                session = %key.session_id,
                web_ui = %key.name,
                %error,
                "web UI relay task failed to stop"
            );
        }
        if clear_exposed_url
            && let Err(error) =
                persist_exposed_url(self.sessions_dir.clone(), key.clone(), None).await
        {
            tracing::warn!(
                session = %key.session_id,
                web_ui = %key.name,
                %error,
                "failed to clear exposed web UI URL"
            );
        }
    }
}

async fn invoke_hook(
    hook_runner: Arc<HookRunner>,
    name: &'static str,
    variables: Vec<(String, String)>,
) -> Result<Option<String>> {
    tokio::task::spawn_blocking(move || {
        let variables = variables
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>();
        hook_runner(name, &variables)
    })
    .await
    .context("failed to join web UI hook")?
}

async fn persist_exposed_url(
    sessions_dir: PathBuf,
    key: RelayKey,
    exposed_url: Option<String>,
) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        store::update_session_in(&sessions_dir, &key.session_id, |session| {
            let Some(registration) = session
                .web_uis
                .iter_mut()
                .find(|registration| registration.name == key.name && registration.url == key.url)
            else {
                return false;
            };
            registration.exposed_url = exposed_url;
            true
        })
    })
    .await
    .context("failed to join web UI exposed URL update")??;
    Ok(())
}

fn relay_url(original_url: &str, port: u16) -> Result<String> {
    let mut url = Url::parse(original_url).context("invalid registered web UI URL")?;
    url.set_host(Some("127.0.0.1"))
        .context("registered web UI URL has an invalid host")?;
    url.set_port(Some(port))
        .map_err(|()| anyhow::anyhow!("web UI relay port is invalid"))?;
    Ok(url.to_string())
}

fn valid_exposed_url(url: String) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| url.to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    use reqwest::Url;
    use serde_json::json;
    use tempfile::TempDir;

    use super::{WebUiRelays, store};
    use crate::commands::agent::types::{Session, WebUiRegistration};

    type HookCalls = Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>;

    fn session() -> Session {
        serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "status": "running",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "web_uis": [
                {
                    "name": "Design editor",
                    "url": "http://127.0.0.1:49121/design/sample/"
                },
                {
                    "name": "Preview",
                    "url": "http://localhost:49122/preview/"
                }
            ]
        }))
        .expect("fixture session should deserialize")
    }

    fn hook_runner(
        calls: HookCalls,
    ) -> impl Fn(&str, &[(&str, &str)]) -> anyhow::Result<Option<String>> + Send + Sync {
        move |hook, variables| {
            calls.lock().expect("hook call lock should succeed").push((
                hook.to_owned(),
                variables
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                    .collect(),
            ));
            Ok((hook == "web-ui-expose")
                .then(|| "https://public.example.invalid/relay/".to_owned()))
        }
    }

    #[tokio::test]
    async fn registration_changes_and_shutdown_reconcile_relays_and_hooks() -> anyhow::Result<()> {
        let directory = TempDir::new()?;
        let initial = session();
        store::save_session_to(directory.path(), &initial)?;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut relays = WebUiRelays::with_hook_runner(
            directory.path().to_path_buf(),
            hook_runner(calls.clone()),
        );

        relays.sync().await?;
        let exposed = store::load_session_from(directory.path(), "fixture-session")?
            .expect("session should still exist");
        let first_state = (
            relays.active.len(),
            relays
                .active
                .values()
                .map(|relay| relay.port)
                .collect::<HashSet<_>>()
                .len(),
            exposed
                .web_uis
                .iter()
                .map(|registration| registration.exposed_url.clone())
                .collect::<Vec<_>>(),
        );

        store::update_session_in(directory.path(), "fixture-session", |session| {
            session
                .web_uis
                .retain(|registration| registration.name == "Preview");
            true
        })?;
        relays.sync().await?;
        relays.stop_all().await;
        let final_session = store::load_session_from(directory.path(), "fixture-session")?
            .expect("session should still exist");
        let final_state = (
            relays.active.len(),
            final_session.web_uis,
            calls
                .lock()
                .expect("hook call lock should succeed")
                .iter()
                .map(|(hook, variables)| {
                    let values = variables
                        .iter()
                        .cloned()
                        .collect::<std::collections::BTreeMap<_, _>>();
                    let url = Url::parse(&values["ARMYKNIFE_WEB_UI_URL"])
                        .expect("hook URL should be valid");
                    let port = values["ARMYKNIFE_WEB_UI_PORT"]
                        .parse::<u16>()
                        .expect("hook port should be valid");
                    (
                        hook.clone(),
                        url.path().to_owned(),
                        port == url
                            .port_or_known_default()
                            .expect("URL should include port"),
                        values.get("ARMYKNIFE_WEB_UI_EXPOSED_URL").cloned(),
                    )
                })
                .collect::<Vec<_>>(),
        );

        let mut expected_calls = vec![
            (
                "web-ui-expose".to_owned(),
                "/design/sample/".to_owned(),
                true,
                None,
            ),
            (
                "web-ui-expose".to_owned(),
                "/preview/".to_owned(),
                true,
                None,
            ),
            (
                "web-ui-unexpose".to_owned(),
                "/design/sample/".to_owned(),
                true,
                Some("https://public.example.invalid/relay/".to_owned()),
            ),
            (
                "web-ui-unexpose".to_owned(),
                "/preview/".to_owned(),
                true,
                Some("https://public.example.invalid/relay/".to_owned()),
            ),
        ];
        expected_calls.sort();
        let mut actual_calls = final_state.2.clone();
        actual_calls.sort();

        assert_eq!(
            (first_state, final_state.0, final_state.1, actual_calls),
            (
                (
                    2,
                    2,
                    vec![
                        Some("https://public.example.invalid/relay/".to_owned()),
                        Some("https://public.example.invalid/relay/".to_owned()),
                    ],
                ),
                0,
                vec![WebUiRegistration {
                    name: "Preview".to_owned(),
                    url: "http://localhost:49122/preview/".to_owned(),
                    exposed_url: None,
                }],
                expected_calls,
            ),
        );
        Ok(())
    }

    #[tokio::test]
    async fn ended_sessions_are_unexposed_by_watch_web() -> anyhow::Result<()> {
        let directory = TempDir::new()?;
        let mut session = session();
        session.web_uis.truncate(1);
        store::save_session_to(directory.path(), &session)?;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut relays = WebUiRelays::with_hook_runner(
            directory.path().to_path_buf(),
            hook_runner(calls.clone()),
        );

        relays.sync().await?;
        store::update_session_in(directory.path(), "fixture-session", |session| {
            session.status = crate::commands::agent::types::SessionStatus::Ended;
            true
        })?;
        relays.sync().await?;
        let actual = (
            relays.active.len(),
            calls
                .lock()
                .expect("hook call lock should succeed")
                .iter()
                .map(|(hook, variables)| {
                    let values = variables
                        .iter()
                        .cloned()
                        .collect::<std::collections::BTreeMap<_, _>>();
                    (
                        hook.clone(),
                        Url::parse(&values["ARMYKNIFE_WEB_UI_URL"])
                            .expect("hook URL should be valid")
                            .path()
                            .to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
        );
        relays.stop_all().await;

        let mut calls = actual.1;
        calls.sort();
        assert_eq!(
            (actual.0, calls),
            (
                0,
                vec![
                    ("web-ui-expose".to_owned(), "/design/sample/".to_owned()),
                    ("web-ui-unexpose".to_owned(), "/design/sample/".to_owned()),
                ],
            ),
        );
        Ok(())
    }
}
