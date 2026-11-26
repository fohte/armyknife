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
const HOOK_TIMEOUT: Duration = Duration::from_secs(30);

type HookRunner = dyn Fn(&str, &[(&str, &str)]) -> Result<Option<String>> + Send + Sync;

mod registrations;
mod unexpose_backoff;

use registrations::{
    RelayRegistration, expose_and_record_registration, load_relay_registrations,
    remove_and_save_relay_registration, remove_relay_registration, unexpose_registration,
};
use unexpose_backoff::UnexposeBackoff;

#[cfg(test)]
use registrations::save_relay_registrations;

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
    exposed: bool,
    registration_saved: bool,
    server: JoinHandle<()>,
}

pub(super) struct WebUiRelays {
    sessions_dir: PathBuf,
    hook_runner: Arc<HookRunner>,
    unexpose_backoff: UnexposeBackoff,
    active: HashMap<RelayKey, Relay>,
}

impl WebUiRelays {
    pub(super) fn new(sessions_dir: PathBuf) -> Self {
        Self::with_hook_runner(sessions_dir, |name, env| {
            hooks::run_hook_capture_stdout_with_timeout(name, env, HOOK_TIMEOUT)
        })
    }

    fn with_hook_runner(
        sessions_dir: PathBuf,
        hook_runner: impl Fn(&str, &[(&str, &str)]) -> Result<Option<String>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            sessions_dir,
            hook_runner: Arc::new(hook_runner),
            unexpose_backoff: UnexposeBackoff::default(),
            active: HashMap::new(),
        }
    }

    pub(super) async fn start(self, shutdown: watch::Receiver<bool>) -> JoinHandle<()> {
        tokio::spawn(self.run_periodic(shutdown))
    }

    async fn run_periodic(mut self, mut shutdown: watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(SYNC_INTERVAL);
        interval.tick().await;
        if !*shutdown.borrow()
            && let Err(error) = self.sync().await
        {
            tracing::warn!(%error, "failed to synchronize web UI relays at startup");
        }
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
        let mut registrations = match load_relay_registrations(&self.sessions_dir) {
            Ok(registrations) => Some(registrations),
            Err(error) => {
                tracing::warn!(%error, "failed to load web UI relay registrations");
                None
            }
        };
        if let Some(registrations) = &mut registrations {
            let active_registrations = self
                .active
                .iter()
                .filter(|(_, relay)| relay.exposed)
                .map(|(key, relay)| RelayRegistration::from_relay(key, relay))
                .collect::<HashSet<_>>();
            let active_ports = self
                .active
                .values()
                .map(|relay| relay.port)
                .collect::<HashSet<_>>();
            let stale_registrations = registrations
                .iter()
                .filter(|registration| !active_registrations.contains(*registration))
                .cloned()
                .collect::<Vec<_>>();
            self.unexpose_backoff
                .retain_for(&stale_registrations.iter().cloned().collect());
            for registration in stale_registrations {
                if active_ports.contains(&registration.port) {
                    self.unexpose_backoff.remove(&registration);
                    // The hook removes by port, so an old record must not target a live relay.
                    if let Err(error) = remove_and_save_relay_registration(
                        &self.sessions_dir,
                        registrations,
                        &registration,
                    ) {
                        tracing::warn!(
                            session = %registration.session_id,
                            web_ui = %registration.name,
                            %error,
                            "failed to remove conflicting web UI relay registration"
                        );
                    }
                    continue;
                }
                if !self
                    .unexpose_backoff
                    .should_attempt(&registration, std::time::Instant::now())
                {
                    continue;
                }
                match unexpose_registration(self.hook_runner.clone(), &registration).await {
                    Ok(()) => {
                        self.unexpose_backoff.remove(&registration);
                        if let Err(error) = remove_and_save_relay_registration(
                            &self.sessions_dir,
                            registrations,
                            &registration,
                        ) {
                            tracing::warn!(
                                session = %registration.session_id,
                                web_ui = %registration.name,
                                %error,
                                "failed to save web UI relay registrations after cleanup"
                            );
                        }
                    }
                    Err(error) => {
                        self.unexpose_backoff
                            .record_failure(registration.clone(), std::time::Instant::now());
                        tracing::warn!(
                            session = %registration.session_id,
                            web_ui = %registration.name,
                            %error,
                            "failed to unexpose stale web UI relay"
                        );
                    }
                }
            }
        }

        let sessions_dir = self.sessions_dir.clone();
        let sessions =
            tokio::task::spawn_blocking(move || store::list_all_sessions_from(&sessions_dir))
                .await
                .context("failed to join web UI session lookup")??;
        let mut desired = HashMap::new();
        for session in sessions
            .iter()
            .filter(|session| session.status != SessionStatus::Ended)
        {
            for registration in &session.web_uis {
                if super::super::web_ui::local_web_ui_port(&registration.url).is_ok() {
                    desired.insert(
                        RelayKey::new(&session.session_id, registration),
                        registration.clone(),
                    );
                } else {
                    let exposed_url = direct_remote_url(&registration.url);
                    if registration.exposed_url != exposed_url
                        && let Err(error) = persist_exposed_url(
                            self.sessions_dir.clone(),
                            RelayKey::new(&session.session_id, registration),
                            exposed_url,
                        )
                        .await
                    {
                        tracing::warn!(
                            session = %session.session_id,
                            web_ui = %registration.name,
                            %error,
                            "failed to save direct web UI URL"
                        );
                    }
                }
            }
        }
        let desired_keys = desired.keys().cloned().collect::<HashSet<_>>();
        let stale_keys = self
            .active
            .keys()
            .filter(|key| {
                !desired_keys.contains(*key)
                    || self
                        .active
                        .get(*key)
                        .is_some_and(|relay| relay.server.is_finished())
            })
            .cloned()
            .collect::<Vec<_>>();

        for key in stale_keys {
            if let Some(relay) = self.active.remove(&key) {
                self.stop_relay(&key, relay, true).await;
            }
        }

        for (key, registration) in desired {
            if let Some(mut relay) = self.active.remove(&key) {
                let previous_exposed_url = relay.exposed_url.clone();
                expose_and_record_registration(
                    &self.sessions_dir,
                    self.hook_runner.clone(),
                    &key,
                    &mut relay,
                )
                .await;
                let exposed_url = relay.exposed_url.clone();
                self.active.insert(key.clone(), relay);
                if previous_exposed_url != exposed_url
                    && let Err(error) =
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
                continue;
            }
            match self.start_relay(&key, &registration).await {
                Ok(mut relay) => {
                    expose_and_record_registration(
                        &self.sessions_dir,
                        self.hook_runner.clone(),
                        &key,
                        &mut relay,
                    )
                    .await;
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
        let client = web_ui_proxy::new_client()?;
        let session_id = key.session_id.clone();
        let web_ui_name = key.name.clone();
        let server = tokio::spawn(async move {
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

        Ok(Relay {
            url,
            port,
            exposed_url: None,
            exposed: false,
            registration_saved: false,
            server,
        })
    }

    async fn stop_all(&mut self) {
        let relays = self.active.drain().collect::<Vec<_>>();
        for (key, relay) in relays {
            self.stop_relay(&key, relay, true).await;
        }
    }

    async fn stop_relay(&mut self, key: &RelayKey, relay: Relay, clear_exposed_url: bool) {
        let registration = RelayRegistration::from_relay(key, &relay);
        let unexpose_result = unexpose_registration(self.hook_runner.clone(), &registration).await;
        if let Err(error) = &unexpose_result {
            self.unexpose_backoff
                .record_failure(registration.clone(), std::time::Instant::now());
            tracing::warn!(
                session = %key.session_id,
                web_ui = %key.name,
                %error,
                "web-ui-unexpose hook failed"
            );
        }
        if unexpose_result.is_ok() {
            self.unexpose_backoff.remove(&registration);
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
        if unexpose_result.is_ok()
            && let Err(error) = remove_relay_registration(&self.sessions_dir, &registration)
        {
            tracing::warn!(
                session = %key.session_id,
                web_ui = %key.name,
                %error,
                "failed to remove web UI relay registration"
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

fn direct_remote_url(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    let host = parsed.host_str()?;
    if host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
    {
        return None;
    }
    Some(url.to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::sync::{Arc, Mutex};

    use anyhow::Context;
    use reqwest::Url;
    use rstest::{fixture, rstest};
    use serde_json::json;
    use tempfile::TempDir;

    use super::{
        RelayRegistration, WebUiRelays, load_relay_registrations, save_relay_registrations, store,
    };
    use crate::commands::agent::types::{Session, WebUiRegistration};

    type HookCalls = Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>;

    struct PreviousProcess {
        directory: TempDir,
        registration: RelayRegistration,
    }

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

    fn session_with_one_web_ui() -> Session {
        let mut session = session();
        session.web_uis.truncate(1);
        session
    }

    #[fixture]
    fn state_write_failure_directory() -> TempDir {
        let directory = TempDir::new().expect("temporary sessions directory should be created");
        store::save_session_to(directory.path(), &session_with_one_web_ui())
            .expect("fixture session should be saved");
        fs::create_dir(directory.path().join("web_ui_relays.tmp"))
            .expect("relay state temp path should block writes");
        directory
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

    type HookCall = (String, Vec<(String, String)>);

    fn recording_hook_runner(
        calls: tokio::sync::mpsc::UnboundedSender<HookCall>,
        fail_first_unexpose: bool,
        fail_expose: bool,
    ) -> impl Fn(&str, &[(&str, &str)]) -> anyhow::Result<Option<String>> + Send + Sync {
        let fail_first_unexpose = Arc::new(std::sync::atomic::AtomicBool::new(fail_first_unexpose));
        move |hook, variables| {
            calls
                .send((
                    hook.to_owned(),
                    variables
                        .iter()
                        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                        .collect(),
                ))
                .ok();
            if (hook == "web-ui-unexpose"
                && fail_first_unexpose.swap(false, std::sync::atomic::Ordering::SeqCst))
                || (hook == "web-ui-expose" && fail_expose)
            {
                anyhow::bail!("fixture hook failed");
            }
            Ok((hook == "web-ui-expose")
                .then(|| "https://public.example.invalid/relay/".to_owned()))
        }
    }

    async fn start_relay_manager(
        directory: &TempDir,
        fail_first_unexpose: bool,
        fail_expose: bool,
    ) -> (
        tokio::sync::watch::Sender<bool>,
        tokio::task::JoinHandle<()>,
        tokio::sync::mpsc::UnboundedReceiver<HookCall>,
    ) {
        let (hook_sender, hook_receiver) = tokio::sync::mpsc::unbounded_channel();
        let relays = WebUiRelays::with_hook_runner(
            directory.path().to_path_buf(),
            recording_hook_runner(hook_sender, fail_first_unexpose, fail_expose),
        );
        let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(false);
        let manager = relays.start(shutdown_receiver).await;
        (shutdown_sender, manager, hook_receiver)
    }

    async fn next_hook_call(
        calls: &mut tokio::sync::mpsc::UnboundedReceiver<HookCall>,
    ) -> anyhow::Result<HookCall> {
        tokio::time::timeout(std::time::Duration::from_secs(5), calls.recv())
            .await?
            .context("expected a hook call")
    }

    async fn wait_for_saved_registration(directory: &TempDir) -> anyhow::Result<RelayRegistration> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(registration) = load_relay_registrations(directory.path())?
                    .into_iter()
                    .next()
                {
                    return Ok::<RelayRegistration, anyhow::Error>(registration);
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await?
    }

    fn summarize_hook_calls(calls: &[HookCall]) -> Vec<(String, Option<String>)> {
        calls
            .iter()
            .map(|(hook, variables)| {
                (
                    hook.clone(),
                    variables
                        .iter()
                        .find(|(name, _)| name == "ARMYKNIFE_WEB_UI_EXPOSED_URL")
                        .map(|(_, value)| value.clone()),
                )
            })
            .collect()
    }

    fn summarize_registration(
        registration: RelayRegistration,
    ) -> (String, String, String, bool, Option<String>) {
        let url = Url::parse(&registration.url).expect("relay URL should be valid");
        (
            registration.session_id,
            registration.name,
            url.path().to_owned(),
            url.port() == Some(registration.port),
            registration.exposed_url,
        )
    }

    #[fixture]
    async fn previous_process() -> PreviousProcess {
        let directory = TempDir::new().expect("temporary sessions directory should be created");
        store::save_session_to(directory.path(), &session_with_one_web_ui())
            .expect("fixture session should be saved");
        let mut previous =
            WebUiRelays::with_hook_runner(directory.path().to_path_buf(), |_, _| Ok(None));
        previous.sync().await.expect("previous relay should start");
        let registration = load_relay_registrations(directory.path())
            .expect("previous relay registration should load")
            .first()
            .cloned()
            .expect("previous relay should be recorded without a public URL");
        previous
            .active
            .values()
            .next()
            .expect("previous relay should be active")
            .server
            .abort();
        drop(previous);
        PreviousProcess {
            directory,
            registration,
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

    #[rstest]
    #[tokio::test]
    async fn startup_unexposes_previous_process_relays_and_keeps_current_registration(
        #[future] previous_process: PreviousProcess,
    ) -> anyhow::Result<()> {
        let previous_process = previous_process.await;
        let directory = previous_process.directory.path();
        let previous_registration = previous_process.registration;

        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut current =
            WebUiRelays::with_hook_runner(directory.to_path_buf(), hook_runner(calls.clone()));
        current.sync().await?;
        let current_registration = current
            .active
            .iter()
            .map(|(key, relay)| RelayRegistration::from_relay(key, relay))
            .next()
            .context("current relay should be active")?;
        current.sync().await?;

        let hook_calls = calls
            .lock()
            .expect("hook call lock should succeed")
            .iter()
            .map(|(hook, variables)| {
                (
                    hook.clone(),
                    variables
                        .iter()
                        .cloned()
                        .collect::<std::collections::BTreeMap<_, _>>(),
                )
            })
            .collect::<Vec<_>>();
        let registrations = load_relay_registrations(directory)?;
        current.stop_all().await;

        assert_eq!(
            (hook_calls, registrations),
            (
                vec![
                    (
                        "web-ui-unexpose".to_owned(),
                        [
                            ("ARMYKNIFE_WEB_UI_URL".to_owned(), previous_registration.url),
                            (
                                "ARMYKNIFE_WEB_UI_PORT".to_owned(),
                                previous_registration.port.to_string(),
                            ),
                            (
                                "ARMYKNIFE_WEB_UI_EXPOSED_URL".to_owned(),
                                previous_registration.exposed_url.unwrap_or_default(),
                            ),
                        ]
                        .into(),
                    ),
                    (
                        "web-ui-expose".to_owned(),
                        [
                            (
                                "ARMYKNIFE_WEB_UI_URL".to_owned(),
                                current_registration.url.clone()
                            ),
                            (
                                "ARMYKNIFE_WEB_UI_PORT".to_owned(),
                                current_registration.port.to_string(),
                            ),
                        ]
                        .into(),
                    ),
                ],
                vec![current_registration],
            ),
        );
        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn failed_stale_unexpose_waits_for_backoff_before_retrying(
        #[future] previous_process: PreviousProcess,
    ) -> anyhow::Result<()> {
        let previous_process = previous_process.await;
        let directory = previous_process.directory.path();
        let previous_registration = previous_process.registration;

        let fail_first_unexpose = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut current = WebUiRelays::with_hook_runner(directory.to_path_buf(), {
            let fail_first_unexpose = fail_first_unexpose.clone();
            let calls = calls.clone();
            move |hook, variables| {
                calls.lock().expect("hook call lock should succeed").push((
                    hook.to_owned(),
                    variables
                        .iter()
                        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                        .collect::<Vec<_>>(),
                ));
                if hook == "web-ui-unexpose"
                    && fail_first_unexpose.swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    anyhow::bail!("fixture hook failed");
                }
                Ok((hook == "web-ui-expose")
                    .then(|| "https://public.example.invalid/relay/".to_owned()))
            }
        });
        current.sync().await?;
        let current_registration = current
            .active
            .iter()
            .map(|(key, relay)| RelayRegistration::from_relay(key, relay))
            .next()
            .context("current relay should be active")?;
        let after_failure = load_relay_registrations(directory)?;
        current.sync().await?;
        let after_immediate_sync = (
            load_relay_registrations(directory)?,
            calls.lock().expect("hook call lock should succeed").len(),
        );
        let hook_names = calls
            .lock()
            .expect("hook call lock should succeed")
            .iter()
            .map(|(hook, _)| hook.clone())
            .collect::<Vec<_>>();
        current.stop_all().await;

        assert_eq!(
            (after_failure, after_immediate_sync, hook_names,),
            (
                vec![previous_registration.clone(), current_registration.clone()],
                (
                    vec![previous_registration.clone(), current_registration.clone()],
                    2
                ),
                vec!["web-ui-unexpose".to_owned(), "web-ui-expose".to_owned(),],
            ),
        );
        Ok(())
    }

    #[tokio::test]
    async fn failed_active_unexpose_waits_for_backoff_on_the_next_sync() -> anyhow::Result<()> {
        let directory = TempDir::new()?;
        store::save_session_to(directory.path(), &session_with_one_web_ui())?;
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let mut relays = WebUiRelays::with_hook_runner(
            directory.path().to_path_buf(),
            recording_hook_runner(sender, true, false),
        );

        relays.sync().await?;
        let registration = load_relay_registrations(directory.path())?;
        store::update_session_in(directory.path(), "fixture-session", |session| {
            session.status = crate::commands::agent::types::SessionStatus::Ended;
            true
        })?;
        relays.sync().await?;
        let after_failure = load_relay_registrations(directory.path())?;
        relays.sync().await?;
        let after_immediate_sync = load_relay_registrations(directory.path())?;
        let mut hook_calls = Vec::new();
        while let Ok(call) = receiver.try_recv() {
            hook_calls.push(call);
        }
        relays.stop_all().await;

        assert_eq!(
            (
                after_failure,
                after_immediate_sync,
                summarize_hook_calls(&hook_calls),
            ),
            (
                registration.clone(),
                registration,
                vec![
                    ("web-ui-expose".to_owned(), None),
                    (
                        "web-ui-unexpose".to_owned(),
                        Some("https://public.example.invalid/relay/".to_owned()),
                    ),
                ],
            ),
        );
        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn failed_relay_state_write_retries_exposure_on_next_sync(
        state_write_failure_directory: TempDir,
    ) -> anyhow::Result<()> {
        let directory = state_write_failure_directory;
        let (shutdown, manager, mut calls) = start_relay_manager(&directory, false, false).await;
        let mut actual_calls = vec![
            next_hook_call(&mut calls).await?,
            next_hook_call(&mut calls).await?,
        ];
        fs::remove_dir(directory.path().join("web_ui_relays.tmp"))?;
        actual_calls.push(next_hook_call(&mut calls).await?);
        let saved_registration = wait_for_saved_registration(&directory).await?;
        shutdown.send_replace(true);
        actual_calls.push(next_hook_call(&mut calls).await?);
        tokio::time::timeout(std::time::Duration::from_secs(5), manager).await??;
        let remaining_registrations = load_relay_registrations(directory.path())?;

        assert_eq!(
            (
                summarize_hook_calls(&actual_calls),
                summarize_registration(saved_registration),
                remaining_registrations,
            ),
            (
                vec![
                    ("web-ui-expose".to_owned(), None),
                    (
                        "web-ui-unexpose".to_owned(),
                        Some("https://public.example.invalid/relay/".to_owned()),
                    ),
                    ("web-ui-expose".to_owned(), None),
                    (
                        "web-ui-unexpose".to_owned(),
                        Some("https://public.example.invalid/relay/".to_owned()),
                    ),
                ],
                (
                    "fixture-session".to_owned(),
                    "Design editor".to_owned(),
                    "/design/sample/".to_owned(),
                    true,
                    Some("https://public.example.invalid/relay/".to_owned()),
                ),
                vec![],
            ),
        );
        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn failed_registration_rollback_preserves_exposed_url_for_shutdown(
        state_write_failure_directory: TempDir,
    ) -> anyhow::Result<()> {
        let directory = state_write_failure_directory;
        let (shutdown, manager, mut calls) = start_relay_manager(&directory, true, false).await;
        let mut actual_calls = vec![
            next_hook_call(&mut calls).await?,
            next_hook_call(&mut calls).await?,
        ];
        fs::remove_dir(directory.path().join("web_ui_relays.tmp"))?;
        let saved_registration = wait_for_saved_registration(&directory).await?;
        shutdown.send_replace(true);
        actual_calls.push(next_hook_call(&mut calls).await?);
        tokio::time::timeout(std::time::Duration::from_secs(5), manager).await??;
        let remaining_registrations = load_relay_registrations(directory.path())?;

        assert_eq!(
            (
                summarize_hook_calls(&actual_calls),
                summarize_registration(saved_registration),
                remaining_registrations,
            ),
            (
                vec![
                    ("web-ui-expose".to_owned(), None),
                    (
                        "web-ui-unexpose".to_owned(),
                        Some("https://public.example.invalid/relay/".to_owned()),
                    ),
                    (
                        "web-ui-unexpose".to_owned(),
                        Some("https://public.example.invalid/relay/".to_owned()),
                    ),
                ],
                (
                    "fixture-session".to_owned(),
                    "Design editor".to_owned(),
                    "/design/sample/".to_owned(),
                    true,
                    Some("https://public.example.invalid/relay/".to_owned()),
                ),
                vec![],
            ),
        );
        Ok(())
    }

    #[tokio::test]
    async fn failed_expose_retries_on_the_same_relay_until_shutdown() -> anyhow::Result<()> {
        let directory = TempDir::new()?;
        store::save_session_to(directory.path(), &session_with_one_web_ui())?;
        let (shutdown, manager, mut calls) = start_relay_manager(&directory, false, true).await;
        let first_expose = next_hook_call(&mut calls).await?;
        let second_expose = next_hook_call(&mut calls).await?;
        let first_port = first_expose
            .1
            .iter()
            .find(|(name, _)| name == "ARMYKNIFE_WEB_UI_PORT")
            .map(|(_, value)| value);
        let second_port = second_expose
            .1
            .iter()
            .find(|(name, _)| name == "ARMYKNIFE_WEB_UI_PORT")
            .map(|(_, value)| value);
        shutdown.send_replace(true);
        let shutdown_unexpose = next_hook_call(&mut calls).await?;
        tokio::time::timeout(std::time::Duration::from_secs(5), manager).await??;

        assert_eq!(
            (
                first_expose.0.clone(),
                second_expose.0.clone(),
                first_port.is_some() && first_port == second_port,
                summarize_hook_calls(&[first_expose, second_expose, shutdown_unexpose]),
            ),
            (
                "web-ui-expose".to_owned(),
                "web-ui-expose".to_owned(),
                true,
                vec![
                    ("web-ui-expose".to_owned(), None),
                    ("web-ui-expose".to_owned(), None),
                    ("web-ui-unexpose".to_owned(), Some(String::new())),
                ],
            ),
        );
        Ok(())
    }

    #[tokio::test]
    async fn unreadable_relay_state_does_not_block_sync_and_is_replaced() -> anyhow::Result<()> {
        let directory = TempDir::new()?;
        store::save_session_to(directory.path(), &session_with_one_web_ui())?;
        fs::write(
            directory.path().join("web_ui_relays.state"),
            b"invalid state",
        )?;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut relays = WebUiRelays::with_hook_runner(
            directory.path().to_path_buf(),
            hook_runner(calls.clone()),
        );

        relays.sync().await?;
        let active_registration = relays
            .active
            .iter()
            .map(|(key, relay)| RelayRegistration::from_relay(key, relay))
            .next()
            .context("relay should start despite malformed state")?;
        let actual = (
            load_relay_registrations(directory.path())?,
            calls
                .lock()
                .expect("hook call lock should succeed")
                .iter()
                .map(|(hook, _)| hook.clone())
                .collect::<Vec<_>>(),
        );
        relays.stop_all().await;

        assert_eq!(
            actual,
            (vec![active_registration], vec!["web-ui-expose".to_owned()],),
        );
        Ok(())
    }

    #[tokio::test]
    async fn relay_state_read_error_does_not_block_sync_or_replace_the_state_path()
    -> anyhow::Result<()> {
        let directory = TempDir::new()?;
        store::save_session_to(directory.path(), &session_with_one_web_ui())?;
        fs::create_dir(directory.path().join("web_ui_relays.state"))?;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut relays = WebUiRelays::with_hook_runner(
            directory.path().to_path_buf(),
            hook_runner(calls.clone()),
        );

        relays.sync().await?;
        let hook_names = calls
            .lock()
            .expect("hook call lock should succeed")
            .iter()
            .map(|(hook, _)| hook.clone())
            .collect::<Vec<_>>();
        let actual = (
            relays.active.len(),
            directory.path().join("web_ui_relays.state").is_dir(),
            hook_names,
        );
        relays.stop_all().await;

        assert_eq!(
            actual,
            (
                1,
                true,
                vec!["web-ui-expose".to_owned(), "web-ui-unexpose".to_owned()],
            ),
        );
        Ok(())
    }

    #[tokio::test]
    async fn stale_registration_with_active_port_is_removed_without_unexposing()
    -> anyhow::Result<()> {
        let directory = TempDir::new()?;
        store::save_session_to(directory.path(), &session_with_one_web_ui())?;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut relays = WebUiRelays::with_hook_runner(
            directory.path().to_path_buf(),
            hook_runner(calls.clone()),
        );

        relays.sync().await?;
        let current_registration = relays
            .active
            .iter()
            .map(|(key, relay)| RelayRegistration::from_relay(key, relay))
            .next()
            .context("current relay should be active")?;
        let mut stale_registration = current_registration.clone();
        stale_registration.session_id = "stale-fixture-session".to_owned();
        stale_registration.name = "Old preview".to_owned();
        stale_registration.exposed_url = Some("https://old.example.invalid/relay/".to_owned());
        save_relay_registrations(
            directory.path(),
            &[stale_registration, current_registration.clone()],
        )?;

        relays.sync().await?;
        let actual = (
            calls
                .lock()
                .expect("hook call lock should succeed")
                .iter()
                .map(|(hook, _)| hook.clone())
                .collect::<Vec<_>>(),
            load_relay_registrations(directory.path())?,
        );
        relays.stop_all().await;

        assert_eq!(
            actual,
            (vec!["web-ui-expose".to_owned()], vec![current_registration],),
        );
        Ok(())
    }

    #[tokio::test]
    async fn ended_sessions_are_unexposed_by_watch_web() -> anyhow::Result<()> {
        let directory = TempDir::new()?;
        let session = session_with_one_web_ui();
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

    #[rstest]
    #[case::missing(ExposeBehavior::Missing, None)]
    #[case::empty(ExposeBehavior::Empty, None)]
    #[case::invalid(ExposeBehavior::Invalid, None)]
    #[case::failure(ExposeBehavior::Failure, None)]
    #[case::valid(ExposeBehavior::Valid, Some("https://public.example.invalid/relay/"))]
    #[tokio::test]
    async fn hook_output_controls_the_stored_remote_url(
        #[case] behavior: ExposeBehavior,
        #[case] expected_url: Option<&str>,
    ) -> anyhow::Result<()> {
        let directory = TempDir::new()?;
        let session = session_with_one_web_ui();
        store::save_session_to(directory.path(), &session)?;
        let mut relays =
            WebUiRelays::with_hook_runner(directory.path().to_path_buf(), move |hook, _| {
                if hook != "web-ui-expose" {
                    return Ok(None);
                }
                match behavior {
                    ExposeBehavior::Missing => Ok(None),
                    ExposeBehavior::Empty => Ok(Some(String::new())),
                    ExposeBehavior::Invalid => Ok(Some("file:///invalid".to_owned())),
                    ExposeBehavior::Failure => anyhow::bail!("fixture hook failed"),
                    ExposeBehavior::Valid => {
                        Ok(Some("https://public.example.invalid/relay/".to_owned()))
                    }
                }
            });

        relays.sync().await?;
        let stored_url = store::load_session_from(directory.path(), "fixture-session")?
            .expect("session should still exist")
            .web_uis[0]
            .exposed_url
            .clone();
        let relay_count = relays.active.len();
        relays.stop_all().await;

        assert_eq!((relay_count, stored_url.as_deref()), (1, expected_url),);
        Ok(())
    }

    #[derive(Clone, Copy)]
    enum ExposeBehavior {
        Missing,
        Empty,
        Invalid,
        Failure,
        Valid,
    }

    #[tokio::test]
    async fn finished_relay_is_unexposed_and_restarted() -> anyhow::Result<()> {
        let directory = TempDir::new()?;
        let session = session_with_one_web_ui();
        store::save_session_to(directory.path(), &session)?;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut relays = WebUiRelays::with_hook_runner(
            directory.path().to_path_buf(),
            hook_runner(calls.clone()),
        );

        relays.sync().await?;
        relays
            .active
            .values()
            .next()
            .expect("relay should be running")
            .server
            .abort();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !relays
                .active
                .values()
                .next()
                .expect("relay should remain registered")
                .server
                .is_finished()
            {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        relays.sync().await?;
        let restarted_count = relays.active.len();
        let call_names = calls
            .lock()
            .expect("hook call lock should succeed")
            .iter()
            .map(|(hook, _)| hook.clone())
            .collect::<Vec<_>>();
        relays.stop_all().await;

        assert_eq!(
            (restarted_count, call_names),
            (
                1,
                vec![
                    "web-ui-expose".to_owned(),
                    "web-ui-unexpose".to_owned(),
                    "web-ui-expose".to_owned(),
                ],
            ),
        );
        Ok(())
    }

    #[tokio::test]
    async fn manager_starts_before_expose_hook_finishes() -> anyhow::Result<()> {
        let directory = TempDir::new()?;
        let session = session_with_one_web_ui();
        store::save_session_to(directory.path(), &session)?;
        let (entered_sender, entered_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let release_receiver = Arc::new(Mutex::new(release_receiver));
        let relays = WebUiRelays::with_hook_runner(directory.path().to_path_buf(), move |_, _| {
            entered_sender.send(()).ok();
            release_receiver
                .lock()
                .expect("hook release lock should succeed")
                .recv()?;
            Ok(None)
        });
        let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(false);
        let manager = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            relays.start(shutdown_receiver),
        )
        .await?;

        tokio::task::spawn_blocking(move || {
            entered_receiver.recv_timeout(std::time::Duration::from_secs(1))
        })
        .await??;
        shutdown_sender.send_replace(true);
        release_sender.send(()).ok();
        release_sender.send(()).ok();
        tokio::time::timeout(std::time::Duration::from_secs(1), manager).await??;

        Ok(())
    }

    #[tokio::test]
    async fn remote_crit_urls_keep_their_registered_target() -> anyhow::Result<()> {
        let directory = TempDir::new()?;
        let mut session = session_with_one_web_ui();
        session.web_uis[0].url = "https://review.example.test:9443/review/item".to_owned();
        store::save_session_to(directory.path(), &session)?;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut relays = WebUiRelays::with_hook_runner(
            directory.path().to_path_buf(),
            hook_runner(calls.clone()),
        );

        relays.sync().await?;
        let stored_url = store::load_session_from(directory.path(), "fixture-session")?
            .expect("session should still exist")
            .web_uis[0]
            .exposed_url
            .clone();
        let hook_calls = calls.lock().expect("hook call lock should succeed").clone();
        relays.stop_all().await;

        assert_eq!(
            (relays.active.len(), stored_url, hook_calls),
            (
                0,
                Some("https://review.example.test:9443/review/item".to_owned()),
                Vec::new(),
            ),
        );
        Ok(())
    }
}
