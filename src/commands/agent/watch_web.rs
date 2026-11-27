mod actions;
mod assets;
mod attachments;
mod codex_model_catalog;
mod directories;
mod events;
pub(super) mod file_attachments;
mod freeze_profiler;
mod image_attachments;
mod interrupted;
mod limits;
mod messages;
mod model_options;
mod paths;
mod restart;
mod review_document;
mod rewind;
mod security;
mod session_enrichment;
mod session_state;
mod session_store_watcher;
mod sessions;
mod stale_session_cleanup;
mod usage;
mod web_ui_proxy;
mod web_ui_relays;

use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::LazyLock;
use tokio::sync::watch;

use anyhow::Result;
use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::any;
use clap::Args;
use include_dir::Dir;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;

use super::browser;
use super::error::CcError;
use crate::commands::agent::store;
use crate::commands::agent::types::Session;
use crate::shared::env_var::EnvVars;
use session_store_watcher::SessionStoreWatcher;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WatchWebArgs {
    /// Port to listen on (0 lets the OS choose an available port)
    #[arg(long, default_value_t = LISTEN_PORT)]
    port: u16,
    /// Do not open the web UI in a browser
    #[arg(long)]
    no_open: bool,
}

const LISTEN_PORT: u16 = 4317;
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(super) static SERVER_INSTANCE_ID: LazyLock<String> =
    LazyLock::new(|| uuid::Uuid::new_v4().to_string());

fn is_invalid_session_id_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<CcError>()
        .is_some_and(|error| matches!(error, CcError::InvalidSessionId(_)))
}

fn ensure_port_allowed(port: u16, own_session_id: Option<&str>) -> Result<()> {
    if port == LISTEN_PORT && own_session_id.is_some() {
        anyhow::bail!(
            "Port {LISTEN_PORT} is reserved for the production watch-web server; use `--port 0` or another port."
        );
    }

    Ok(())
}

async fn start_web_ui_relay_manager_if_allowed<F, Fut, T>(
    is_agent_session: bool,
    start_manager: F,
) -> Option<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    if is_agent_session {
        return None;
    }

    Some(start_manager().await)
}

fn restart_args_with_port(args: impl IntoIterator<Item = OsString>, port: u16) -> Vec<OsString> {
    let mut args = args.into_iter().peekable();
    let mut restart_args = Vec::new();
    while let Some(arg) = args.next() {
        if arg == OsStr::new("--port") {
            args.next();
        } else if arg.to_string_lossy().starts_with("--port=") {
            continue;
        } else {
            restart_args.push(arg);
        }
    }
    restart_args.push(OsString::from("--port"));
    restart_args.push(OsString::from(port.to_string()));
    restart_args
}

async fn bind_listener(port: u16) -> Result<(TcpListener, SocketAddr)> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await?;
    let address = listener.local_addr()?;
    Ok((listener, address))
}

fn watch_web_url(address: SocketAddr) -> String {
    format!("http://{address}")
}

fn open_browser_on_startup(
    args: &WatchWebArgs,
    is_restarted_process: bool,
    url: &str,
    open_url: impl FnOnce(&str) -> std::io::Result<()>,
) {
    if !args.no_open
        && !is_restarted_process
        && let Err(error) = open_url(url)
    {
        eprintln!("Could not open a browser automatically: {error}");
    }
}

#[derive(Clone)]
pub(super) struct EnrichmentUpdates {
    sender: watch::Sender<()>,
}

impl EnrichmentUpdates {
    fn new() -> Self {
        let (sender, _) = watch::channel(());
        Self { sender }
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<()> {
        self.sender.subscribe()
    }

    pub(super) fn notify(&self) {
        self.sender.send_replace(());
    }
}

pub async fn run(args: &WatchWebArgs) -> Result<()> {
    let own_session_id = EnvVars::load().own_session_id();
    ensure_port_allowed(args.port, own_session_id.as_deref())?;
    let is_agent_session = own_session_id.is_some();
    let sessions_dir = store::sessions_dir()?;
    let session_store_watcher = SessionStoreWatcher::start_or_warn(&sessions_dir);
    let session_store_watcher_available = session_store_watcher.is_some();

    let executable = std::env::current_exe()?;
    let mut executable_identity = restart::ExecutableIdentity::read(&executable)?;
    let mut executable_watcher =
        restart::create_watcher_or_warn(|| restart::ExecutableWatcher::new(&executable));
    let mut startup_replacement_pending = executable_watcher.is_some()
        && restart::ExecutableIdentity::read(&executable)? != executable_identity;
    let (listener, address) = bind_listener(args.port).await?;
    ensure_port_allowed(address.port(), own_session_id.as_deref())?;
    let restart_args = restart_args_with_port(std::env::args_os().skip(1), address.port());
    let url = watch_web_url(address);
    println!("Agent Watch Web UI: {url}");
    open_browser_on_startup(
        args,
        restart::is_restarted_process(),
        &url,
        browser::open_url,
    );

    let mut listener = listener;
    loop {
        let (restart_sender, restart_receiver) = watch::channel(false);
        let mut shutdown_receiver = restart_sender.subscribe();
        let relay_sessions_dir = sessions_dir.clone();
        let session_store_events =
            SessionStoreWatcher::subscribe_or_closed(session_store_watcher.as_ref());
        let watch_web_server = async move {
            let (relay_shutdown_sender, relay_shutdown_receiver) = watch::channel(false);
            // Relay registrations are shared, so a development process must not reconcile them.
            let relay_task = start_web_ui_relay_manager_if_allowed(is_agent_session, || {
                web_ui_relays::WebUiRelays::new(relay_sessions_dir)
                    .start(relay_shutdown_receiver.clone())
            })
            .await;
            let cleanup_task = stale_session_cleanup::start(relay_shutdown_receiver);
            let shutdown_signal_sender = relay_shutdown_sender.clone();
            let result = axum::serve(
                listener,
                app(
                    restart_receiver,
                    session_store_events,
                    session_store_watcher_available,
                ),
            )
            .with_graceful_shutdown(async move {
                tokio::select! {
                    _ = shutdown_receiver.changed() => {}
                    _ = process_shutdown_signal() => {}
                }
                shutdown_signal_sender.send_replace(true);
            })
            .await;
            relay_shutdown_sender.send_replace(true);
            if let Err(error) = cleanup_task.await {
                tracing::warn!(%error, "stale session cleanup task failed to stop");
            }
            if let Some(relay_task) = relay_task
                && let Err(error) = relay_task.await
            {
                tracing::warn!(%error, "web UI relay manager failed to stop");
            }
            result
        };
        let server = watch_web_server;
        match restart::wait_for_server_or_restart(
            server,
            restart::wait_for_replacement(
                &executable,
                executable_identity,
                &mut startup_replacement_pending,
                executable_watcher
                    .as_mut()
                    .map(|watcher| &mut watcher.events),
            ),
            restart_sender,
            restart::SERVER_SHUTDOWN_TIMEOUT,
        )
        .await?
        {
            restart::ServerOutcome::Exited => return Ok(()),
            restart::ServerOutcome::Restart => {
                match restart::exec_replacement(&executable, executable_identity, &restart_args) {
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("Could not restart the Agent Watch Web UI: {error}");
                        if let Ok(identity) = restart::ExecutableIdentity::read(&executable) {
                            executable_identity = identity;
                        }
                    }
                }
                listener = bind_listener(address.port()).await?.0;
            }
        }
    }
}

async fn process_shutdown_signal() {
    let mut terminate =
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(error) => {
                tracing::warn!(%error, "failed to listen for termination signal");
                return std::future::pending().await;
            }
        };
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                tracing::warn!(%error, "failed to listen for interrupt signal");
                std::future::pending().await
            }
        }
        _ = terminate.recv() => {}
    }
}

fn app(
    restart_signal: watch::Receiver<bool>,
    session_store_events: tokio::sync::broadcast::Receiver<()>,
    session_store_watcher_available: bool,
) -> Router {
    app_with_assets_and_queued_prompt_loaders_and_restart_signal(
        &assets::DIST,
        store::load_session,
        store::watch_web_user_marker,
        messages::RouteLoaders::new(
            |session_id| store::watch_web_queued_prompts_from(&store::sessions_dir()?, session_id),
            |session_id| {
                store::watch_web_peer_out_messages_from(&store::sessions_dir()?, session_id)
            },
            store::mark_watch_web_queued_prompts_resolved,
            store::agent_proposals,
        )
        .with_todo_loader(store::agent_todos)
        .with_permission_request_data_loader(|session_id, request_id| {
            store::permission_request_data(&store::sessions_dir()?, session_id, request_id)
        })
        .with_permission_denied_loader(|session_id| {
            store::permission_denied_operations_from(&store::sessions_dir()?, session_id)
        })
        .with_agent_file_loader(store::agent_files)
        .with_agent_file_storage_root_loader(|| super::file::storage_root().ok())
        .with_background_task_loader(messages::registered_background_tasks)
        .with_session_list_loader(store::list_all_sessions)
        .with_vibe_context(
            crate::shared::config::load_config_or_default,
            crate::commands::agent::vibe::repo_identity_for_path,
        ),
        restart_signal,
        session_store_events,
        session_store_watcher_available,
    )
}

#[cfg(test)]
fn app_with_assets(dist: &'static Dir<'static>) -> Router {
    app_with_assets_and_loader(dist, store::load_session, store::watch_web_user_marker)
}

#[cfg(test)]
fn app_with_assets_and_loader(
    dist: &'static Dir<'static>,
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
    load_user_marker: impl Fn(&str) -> anyhow::Result<Option<String>> + Send + Sync + 'static,
) -> Router {
    app_with_assets_and_loader_and_restart_signal(
        dist,
        load_session,
        load_user_marker,
        watch::channel(false).1,
    )
}

#[cfg(test)]
fn app_with_assets_and_loader_and_restart_signal(
    dist: &'static Dir<'static>,
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
    load_user_marker: impl Fn(&str) -> anyhow::Result<Option<String>> + Send + Sync + 'static,
    restart_signal: watch::Receiver<bool>,
) -> Router {
    app_with_assets_and_queued_prompt_loaders_and_restart_signal(
        dist,
        load_session,
        load_user_marker,
        messages::RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        ),
        restart_signal,
        tokio::sync::broadcast::channel(1).1,
        true,
    )
}

fn app_with_assets_and_queued_prompt_loaders_and_restart_signal(
    dist: &'static Dir<'static>,
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
    load_user_marker: impl Fn(&str) -> anyhow::Result<Option<String>> + Send + Sync + 'static,
    loaders: messages::RouteLoaders,
    restart_signal: watch::Receiver<bool>,
    session_store_events: tokio::sync::broadcast::Receiver<()>,
    session_store_watcher_available: bool,
) -> Router {
    let enrichment_updates = EnrichmentUpdates::new();
    let api = Router::new()
        .merge(actions::router(loaders.clone()))
        .merge(session_state::router())
        .merge(sessions::router(
            enrichment_updates.clone(),
            loaders.clone(),
        ))
        .merge(usage::router())
        .merge(freeze_profiler::router())
        .merge(directories::router())
        .merge(model_options::router())
        .merge(events::router(
            enrichment_updates,
            restart_signal,
            session_store_events,
            session_store_watcher_available,
        ))
        .merge(messages::routes_with_proposals_and_peer_out_messages(
            load_session,
            load_user_marker,
            loaders,
        ))
        .merge(review_document::router())
        .merge(file_attachments::routes())
        .merge(image_attachments::routes())
        .fallback(api_not_found);

    Router::new()
        .nest("/api", api)
        .route("/api/", any(api_not_found))
        .fallback(move |request: axum::extract::Request| async move {
            assets::serve_from(request.uri(), dist, &SERVER_INSTANCE_ID, SERVER_VERSION)
        })
        .layer(axum::middleware::from_fn(security::validate_request))
        .layer(compression_layer())
}

fn compression_layer() -> CompressionLayer {
    CompressionLayer::new().gzip(true)
}

async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "API route not found")
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::header::{CACHE_CONTROL, CONTENT_ENCODING, CONTENT_TYPE, HOST, ORIGIN};
    use axum::http::{Request, Response, StatusCode};
    use axum::response::sse::{Event, Sse};
    use clap::Parser;
    use flate2::read::GzDecoder;
    use futures_util::stream::{self, StreamExt};
    use include_dir::{Dir, include_dir};
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use std::ffi::OsString;
    use std::io::Read;
    use std::net::{IpAddr, Ipv4Addr};
    use tokio::sync::watch;
    use tower::ServiceExt;

    use super::{
        LISTEN_PORT, app, app_with_assets, app_with_assets_and_loader, bind_listener,
        compression_layer, ensure_port_allowed, open_browser_on_startup, restart_args_with_port,
        start_web_ui_relay_manager_if_allowed, watch_web_url,
    };
    use crate::cli::{Cli, Commands};
    use crate::commands::agent::AgentCommands;
    use crate::commands::agent::types::Session;

    static TEST_DIST: Dir<'static> =
        include_dir!("$CARGO_MANIFEST_DIR/src/commands/agent/watch_web/test_assets");

    #[rstest]
    #[case::agent_session_cannot_use_production_port(LISTEN_PORT, Some("fixture-session"), false)]
    #[case::production_port_outside_agent_session(LISTEN_PORT, None, true)]
    #[case::ephemeral_port_in_agent_session(0, Some("fixture-session"), true)]
    #[case::custom_port_in_agent_session(4318, Some("fixture-session"), true)]
    fn watch_web_port_guard_reserves_production_port(
        #[case] port: u16,
        #[case] own_session_id: Option<&str>,
        #[case] expected_is_ok: bool,
    ) {
        assert_eq!(
            ensure_port_allowed(port, own_session_id).is_ok(),
            expected_is_ok
        );
    }

    #[rstest]
    #[case::production_process(false, true)]
    #[case::agent_session(true, false)]
    #[tokio::test]
    async fn agent_sessions_do_not_start_the_shared_web_ui_relay_manager(
        #[case] is_agent_session: bool,
        #[case] expected_to_start: bool,
    ) {
        let mut manager_started = false;
        let manager = start_web_ui_relay_manager_if_allowed(is_agent_session, || async {
            manager_started = true;
        })
        .await;

        assert_eq!(
            (manager.is_some(), manager_started),
            (expected_to_start, expected_to_start)
        );
    }

    #[rstest]
    #[case::uses_default_port(&["a", "agent", "watch-web"], LISTEN_PORT)]
    #[case::accepts_ephemeral_port(&["a", "agent", "watch-web", "--port", "0"], 0)]
    #[case::accepts_custom_port(&["a", "agent", "watch-web", "--port", "4318"], 4318)]
    #[case::accepts_explicit_production_port(&["a", "agent", "watch-web", "--port", "4317"], LISTEN_PORT)]
    fn watch_web_port_cli_parses_arguments(#[case] argv: &[&str], #[case] expected_port: u16) {
        let parsed = Cli::try_parse_from(argv).unwrap();
        let args = match parsed.command {
            Commands::Agent(AgentCommands::WatchWeb(args)) => args,
            _ => panic!("watch-web arguments should parse"),
        };

        assert_eq!(args.port, expected_port);
    }

    #[test]
    fn watch_web_port_cli_rejects_explicit_production_port_in_an_agent_session() {
        let parsed = Cli::try_parse_from(["a", "agent", "watch-web", "--port", "4317"]).unwrap();
        let args = match parsed.command {
            Commands::Agent(AgentCommands::WatchWeb(args)) => args,
            _ => panic!("watch-web arguments should parse"),
        };

        assert_eq!(
            ensure_port_allowed(args.port, Some("fixture-session"))
                .err()
                .map(|error| error.to_string()),
            Some(
                "Port 4317 is reserved for the production watch-web server; use `--port 0` or another port."
                    .to_owned()
            )
        );
    }

    #[tokio::test]
    async fn watch_web_port_zero_binds_loopback_and_reports_the_selected_port() {
        let (_listener, address) = bind_listener(0).await.unwrap();
        let url = watch_web_url(address);

        assert_eq!(
            (address.ip(), address.port() > 0, url),
            (
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                true,
                format!("http://127.0.0.1:{}", address.port())
            )
        );
    }

    #[rstest]
    #[case::inline_port_form(vec![
        OsString::from("agent"),
        OsString::from("watch-web"),
        OsString::from("--port=0"),
        OsString::from("--no-open"),
    ])]
    #[case::separate_port_form(vec![
        OsString::from("agent"),
        OsString::from("watch-web"),
        OsString::from("--port"),
        OsString::from("0"),
        OsString::from("--no-open"),
    ])]
    fn watch_web_port_restart_arguments_keep_the_bound_port(#[case] args: Vec<OsString>) {
        assert_eq!(
            restart_args_with_port(args, 43_210),
            [
                OsString::from("agent"),
                OsString::from("watch-web"),
                OsString::from("--no-open"),
                OsString::from("--port"),
                OsString::from("43210"),
            ]
        );
    }

    #[fixture]
    fn fixture_session() -> Session {
        serde_json::from_value(serde_json::json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "status": "ended",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .unwrap()
    }

    #[rstest]
    #[case::opens_browser_by_default(&["a", "agent", "watch-web"], false, Some("http://127.0.0.1:12345"))]
    #[case::does_not_open_browser_when_requested(
        &["a", "agent", "watch-web", "--no-open"],
        false,
        None
    )]
    #[case::does_not_reopen_browser_after_restart(
        &["a", "agent", "watch-web"],
        true,
        None
    )]
    fn test_no_open_flag(
        #[case] argv: &[&str],
        #[case] is_restarted_process: bool,
        #[case] expected_opened_url: Option<&str>,
    ) {
        let parsed = Cli::try_parse_from(argv).unwrap();
        let args = match parsed.command {
            Commands::Agent(AgentCommands::WatchWeb(args)) => args,
            _ => panic!("watch-web arguments should parse"),
        };
        let mut opened_url = None;
        open_browser_on_startup(
            &args,
            is_restarted_process,
            "http://127.0.0.1:12345",
            |url| {
                opened_url = Some(url.to_owned());
                Ok(())
            },
        );

        assert_eq!(opened_url.as_deref(), expected_opened_url);
    }

    #[rstest]
    #[case::api_root(
        "/api",
        StatusCode::NOT_FOUND,
        Some("text/plain; charset=utf-8"),
        "API route not found"
    )]
    #[case::api_trailing_slash(
        "/api/",
        StatusCode::NOT_FOUND,
        Some("text/plain; charset=utf-8"),
        "API route not found"
    )]
    #[case::unknown_api_path(
        "/api/unimplemented",
        StatusCode::NOT_FOUND,
        Some("text/plain; charset=utf-8"),
        "API route not found"
    )]
    #[case::api_prefix_lookalike(
        "/apifoo",
        StatusCode::OK,
        Some("text/html"),
        include_str!("watch_web/test_assets/index.html")
    )]
    #[tokio::test]
    async fn only_api_paths_use_the_api_fallback(
        #[case] uri: &str,
        #[case] expected_status: StatusCode,
        #[case] expected_content_type: Option<&str>,
        #[case] expected_body: &str,
    ) {
        let response = app_with_assets(&TEST_DIST)
            .oneshot(request("GET", uri, "localhost", None))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response).await,
            (
                expected_status,
                expected_content_type.map(str::to_owned),
                expected_body.to_owned(),
            )
        );
    }

    #[rstest]
    #[case::untrusted_host_get(
        "GET",
        "/",
        "localhost..evil.com",
        None,
        StatusCode::FORBIDDEN,
        None,
        ""
    )]
    #[case::post_without_origin(
        "POST",
        "/api/x",
        "localhost",
        None,
        StatusCode::FORBIDDEN,
        None,
        ""
    )]
    #[case::matching_origin_reaches_api_router(
        "POST",
        "/api/x",
        "localhost",
        Some("http://localhost"),
        StatusCode::NOT_FOUND,
        Some("text/plain; charset=utf-8"),
        "API route not found"
    )]
    #[case::action_route_rejects_missing_origin(
        "POST",
        "/api/sessions/not-found/focus",
        "localhost",
        None,
        StatusCode::FORBIDDEN,
        None,
        ""
    )]
    #[tokio::test]
    async fn request_security_is_applied_by_the_router(
        #[case] method: &str,
        #[case] uri: &str,
        #[case] host: &str,
        #[case] origin: Option<&str>,
        #[case] expected_status: StatusCode,
        #[case] expected_content_type: Option<&str>,
        #[case] expected_body: &str,
    ) {
        let response = app(
            watch::channel(false).1,
            tokio::sync::broadcast::channel(1).1,
            true,
        )
        .oneshot(request(method, uri, host, origin))
        .await
        .unwrap();

        assert_eq!(
            response_snapshot(response).await,
            (
                expected_status,
                expected_content_type.map(str::to_owned),
                expected_body.to_owned(),
            )
        );
    }

    #[rstest]
    #[case::known_session(
        "/api/sessions/fixture-session/messages",
        StatusCode::OK,
        Some("application/json"),
        r#"{"messages":[],"backgroundTasks":[],"start":0,"total":0,"hasUnansweredProposal":false,"activeQuestionIndex":null}"#
    )]
    #[case::unknown_session(
        "/api/sessions/unregistered-session/messages",
        StatusCode::NOT_FOUND,
        None,
        ""
    )]
    #[tokio::test]
    async fn messages_route_returns_session_messages_or_not_found(
        #[case] uri: &str,
        #[case] expected_status: StatusCode,
        #[case] expected_content_type: Option<&str>,
        #[case] expected_body: &str,
        fixture_session: Session,
    ) {
        let router = app_with_assets_and_loader(
            &TEST_DIST,
            move |session_id| Ok((session_id == "fixture-session").then(|| fixture_session.clone())),
            |_| Ok(None),
        );
        let response = router
            .oneshot(request("GET", uri, "localhost", None))
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response).await,
            (
                expected_status,
                expected_content_type.map(str::to_owned),
                expected_body.to_owned(),
            )
        );
    }

    #[rstest]
    #[tokio::test]
    async fn api_responses_are_gzip_encoded(fixture_session: Session) {
        let router = app_with_assets_and_loader(
            &TEST_DIST,
            move |session_id| Ok((session_id == "fixture-session").then(|| fixture_session.clone())),
            |_| Ok(None),
        );

        let compressed_api = router
            .oneshot(compressed_request("/api/sessions/fixture-session/messages"))
            .await
            .unwrap();
        let compressed_api_encoding = compressed_api
            .headers()
            .get(CONTENT_ENCODING)
            .map(|value| value.to_str().unwrap().to_owned());
        let compressed_api_body = to_bytes(compressed_api.into_body(), usize::MAX)
            .await
            .unwrap();

        assert_eq!(
            (
                compressed_api_encoding,
                decode_gzip(&compressed_api_body),
            ),
            (
                Some("gzip".to_owned()),
                br#"{"messages":[],"backgroundTasks":[],"start":0,"total":0,"hasUnansweredProposal":false,"activeQuestionIndex":null}"#.to_vec(),
            ),
        );
    }

    #[rstest]
    #[case::javascript("/assets/app-abcdef12.js", "text/javascript", include_bytes!("watch_web/test_assets/assets/app-abcdef12.js").as_slice())]
    #[case::stylesheet("/assets/app-abcdef12.css", "text/css", include_bytes!("watch_web/test_assets/assets/app-abcdef12.css").as_slice())]
    #[case::index("/", "text/html", include_bytes!("watch_web/test_assets/index.html").as_slice())]
    #[case::spa_fallback("/sessions/fixture-session", "text/html", include_bytes!("watch_web/test_assets/index.html").as_slice())]
    #[tokio::test]
    async fn static_responses_are_gzip_encoded(
        #[case] uri: &str,
        #[case] expected_content_type: &str,
        #[case] expected_body: &[u8],
    ) {
        let response = app_with_assets(&TEST_DIST)
            .oneshot(compressed_request(uri))
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|value| value.to_str().unwrap().to_owned());
        let content_encoding = response
            .headers()
            .get(CONTENT_ENCODING)
            .map(|value| value.to_str().unwrap().to_owned());
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();

        assert_eq!(
            (status, content_type, content_encoding, decode_gzip(&body),),
            (
                StatusCode::OK,
                Some(expected_content_type.to_owned()),
                Some("gzip".to_owned()),
                expected_body.to_vec(),
            ),
        );
    }

    #[rstest]
    #[case::hashed_javascript("/assets/app-abcdef12.js", "public, max-age=31536000, immutable")]
    #[case::hash_contains_hyphen("/assets/app-Ab-d_f12.js", "public, max-age=31536000, immutable")]
    #[case::hashed_stylesheet("/assets/app-abcdef12.css", "public, max-age=31536000, immutable")]
    #[case::unhashed_stylesheet("/assets/app.css", "no-cache")]
    #[case::descriptive_suffix_is_not_a_hash("/assets/logo-illustration.svg", "no-cache")]
    #[case::index_root("/", "no-cache")]
    #[case::direct_index("/index.html", "no-cache")]
    #[case::spa_fallback("/sessions/fixture-session", "no-cache")]
    #[tokio::test]
    async fn static_cache_policy_matches_asset_hashing(
        #[case] uri: &str,
        #[case] expected_cache_control: &str,
    ) {
        let response = app_with_assets(&TEST_DIST)
            .oneshot(request("GET", uri, "localhost", None))
            .await
            .unwrap();
        let cache_control = response
            .headers()
            .get(CACHE_CONTROL)
            .map(|value| value.to_str().unwrap().to_owned());

        assert_eq!(cache_control, Some(expected_cache_control.to_owned()));
    }

    #[tokio::test]
    async fn compression_layer_sends_sse_events_without_compression() {
        let router = axum::Router::new()
            .route(
                "/api/events",
                axum::routing::get(|| async {
                    Sse::new(
                        stream::once(async {
                            Ok::<_, std::convert::Infallible>(
                                Event::default().data("ready".repeat(32)),
                            )
                        })
                        .chain(stream::pending()),
                    )
                }),
            )
            .layer(compression_layer());
        let response = router
            .oneshot(compressed_request("/api/events"))
            .await
            .unwrap();
        let status = response.status();
        let content_encoding = response
            .headers()
            .get(CONTENT_ENCODING)
            .map(|value| value.to_str().unwrap().to_owned());
        let first_chunk = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            response.into_body().into_data_stream().next(),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        let expected_event = format!(
            indoc! {"
                data: {}

            "},
            "ready".repeat(32),
        );

        assert_eq!(
            (
                status,
                content_encoding,
                String::from_utf8(first_chunk.to_vec()).unwrap(),
            ),
            (StatusCode::OK, None, expected_event),
        );
    }

    #[tokio::test]
    async fn matching_origin_reaches_the_action_route() {
        let response = app(
            watch::channel(false).1,
            tokio::sync::broadcast::channel(1).1,
            true,
        )
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/not-found/answer")
                .header(HOST, "localhost")
                .header(ORIGIN, "http://localhost")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"skip":"invalid"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(
            response_snapshot(response).await,
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Some("text/plain; charset=utf-8".to_owned()),
                "Failed to deserialize the JSON body into the target type: skip: invalid type: string \"invalid\", expected a boolean at line 1 column 17".to_owned(),
            )
        );
    }
    fn request(method: &str, uri: &str, host: &str, origin: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(HOST, host);
        if let Some(origin) = origin {
            builder = builder.header(ORIGIN, origin);
        }
        builder.body(Body::empty()).unwrap()
    }

    fn compressed_request(uri: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header(HOST, "localhost")
            .header("accept-encoding", "gzip")
            .body(Body::empty())
            .unwrap()
    }

    fn decode_gzip(body: &[u8]) -> Vec<u8> {
        let mut decoder = GzDecoder::new(body);
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        decoded
    }

    async fn response_snapshot(response: Response<Body>) -> (StatusCode, Option<String>, String) {
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|value| value.to_str().unwrap().to_owned());
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            content_type,
            String::from_utf8(body.to_vec()).unwrap(),
        )
    }
}
