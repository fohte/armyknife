mod actions;
mod assets;
mod attachments;
mod crit_proxy;
mod events;
mod image_attachments;
mod interrupted;
mod limits;
mod messages;
mod restart;
mod security;
mod session_enrichment;
mod session_state;
mod sessions;

use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
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

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WatchWebArgs {
    /// Do not open the web UI in a browser
    #[arg(long)]
    no_open: bool,
}

const LISTEN_PORT: u16 = 4317;
const CRIT_PROXY_PORT: u16 = 4318;

fn is_invalid_session_id_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<CcError>()
        .is_some_and(|error| matches!(error, CcError::InvalidSessionId(_)))
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
    let executable = std::env::current_exe()?;
    let mut executable_identity = restart::ExecutableIdentity::read(&executable)?;
    let restart_args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, LISTEN_PORT));
    let crit_proxy_address = SocketAddr::from((Ipv4Addr::LOCALHOST, CRIT_PROXY_PORT));
    let listener = TcpListener::bind(address).await?;
    let crit_proxy_listener = TcpListener::bind(crit_proxy_address).await?;
    let crit_proxy_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()?;
    let url = format!("http://{address}");
    println!("Agent Watch Web UI: {url}");
    open_browser_on_startup(
        args,
        restart::is_restarted_process(),
        &url,
        browser::open_url,
    );

    let mut listener = listener;
    let mut crit_proxy_listener = crit_proxy_listener;
    let mut restart_retry_interval = restart::POLL_INTERVAL;
    loop {
        let (restart_sender, restart_receiver) = watch::channel(false);
        let mut shutdown_receiver = restart_sender.subscribe();
        let crit_proxy_restart_receiver = restart_receiver.clone();
        let crit_proxy_client = crit_proxy_client.clone();
        let crit_proxy_server = async move {
            axum::serve(
                crit_proxy_listener,
                crit_proxy::router(store::list_all_sessions, crit_proxy_client),
            )
            .with_graceful_shutdown(async move {
                let mut shutdown_receiver = crit_proxy_restart_receiver;
                let _ = shutdown_receiver.changed().await;
            })
            .await
        };
        let watch_web_server = async move {
            axum::serve(listener, app(restart_receiver))
                .with_graceful_shutdown(async move {
                    let _ = shutdown_receiver.changed().await;
                })
                .await
        };
        let server = async move { tokio::try_join!(watch_web_server, crit_proxy_server) };
        tokio::pin!(server);

        tokio::select! {
            result = &mut server => return result.map(|_| ()).map_err(Into::into),
            () = restart::wait_for_replacement(&executable, executable_identity, restart_retry_interval) => {
                restart_sender.send_replace(true);
                server.await?;
                match restart::exec_replacement(&executable, executable_identity, &restart_args) {
                    Ok(_) => restart_retry_interval = restart::POLL_INTERVAL,
                    Err(error) => {
                        eprintln!("Could not restart the Agent Watch Web UI: {error}");
                        if let Ok(identity) = restart::ExecutableIdentity::read(&executable) {
                            executable_identity = identity;
                        }
                        restart_retry_interval = restart::EXEC_RETRY_INTERVAL;
                    }
                }
                listener = TcpListener::bind(address).await?;
                crit_proxy_listener = TcpListener::bind(crit_proxy_address).await?;
            }
        }
    }
}

fn app(restart_signal: watch::Receiver<bool>) -> Router {
    app_with_assets_and_queued_prompt_loaders_and_restart_signal(
        &assets::DIST,
        store::load_session,
        store::watch_web_user_marker,
        messages::RouteLoaders::new(
            |session_id| store::agent_decisions_from(&store::sessions_dir()?, session_id),
            |session_id| store::watch_web_queued_prompts_from(&store::sessions_dir()?, session_id),
            |session_id| {
                store::watch_web_peer_out_messages_from(&store::sessions_dir()?, session_id)
            },
            store::delete_watch_web_queued_prompts,
            store::agent_proposals,
        )
        .with_permission_request_data_loader(|session_id, request_id| {
            store::permission_request_data(&store::sessions_dir()?, session_id, request_id)
        }),
        restart_signal,
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
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        ),
        restart_signal,
    )
}

fn app_with_assets_and_queued_prompt_loaders_and_restart_signal(
    dist: &'static Dir<'static>,
    load_session: impl Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync + 'static,
    load_user_marker: impl Fn(&str) -> anyhow::Result<Option<String>> + Send + Sync + 'static,
    loaders: messages::RouteLoaders,
    restart_signal: watch::Receiver<bool>,
) -> Router {
    let enrichment_updates = EnrichmentUpdates::new();
    let api = Router::new()
        .merge(actions::router(loaders.clone()))
        .merge(session_state::router())
        .merge(sessions::router(
            enrichment_updates.clone(),
            loaders.clone(),
        ))
        .merge(events::router(enrichment_updates, restart_signal))
        .merge(messages::routes_with_proposals_and_peer_out_messages(
            load_session,
            load_user_marker,
            loaders,
        ))
        .merge(image_attachments::routes())
        .fallback(api_not_found);

    Router::new()
        .nest("/api", api)
        .route("/api/", any(api_not_found))
        .fallback(move |request: axum::extract::Request| async move {
            assets::serve_from(request.uri(), dist)
        })
        .layer(axum::middleware::from_fn(security::validate_request))
        .layer(compression_layer())
}

fn compression_layer() -> CompressionLayer {
    CompressionLayer::new().gzip(true).br(true)
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
    use futures_util::stream::{self, StreamExt};
    use include_dir::{Dir, include_dir};
    use indoc::indoc;
    use rstest::rstest;
    use tokio::sync::watch;
    use tower::ServiceExt;

    use super::{
        app, app_with_assets, app_with_assets_and_loader, compression_layer,
        open_browser_on_startup,
    };
    use crate::cli::{Cli, Commands};
    use crate::commands::agent::AgentCommands;
    use crate::commands::agent::types::Session;

    static TEST_DIST: Dir<'static> =
        include_dir!("$CARGO_MANIFEST_DIR/src/commands/agent/watch_web/test_assets");

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
        "<main>fixture content repeated to verify gzip compression and cache revalidation: fixture content repeated to verify gzip compression and cache revalidation: fixture content repeated to verify gzip compression and cache revalidation</main>\n"
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
        "localhost.evil.com",
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
        let response = app(watch::channel(false).1)
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
        r#"{"messages":[],"backgroundTasks":[],"start":0,"total":0,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null}"#
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
    ) {
        let session: Session = serde_json::from_value(serde_json::json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "status": "ended",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .unwrap();
        let router = app_with_assets_and_loader(
            &TEST_DIST,
            move |session_id| Ok((session_id == "fixture-session").then(|| session.clone())),
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

    #[tokio::test]
    async fn watch_web_compresses_api_and_static_responses_and_sets_asset_cache_policy() {
        let session: Session = serde_json::from_value(serde_json::json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "status": "ended",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .unwrap();
        let router = app_with_assets_and_loader(
            &TEST_DIST,
            move |session_id| Ok((session_id == "fixture-session").then(|| session.clone())),
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
                compressed_api_body.starts_with(&[0x1f, 0x8b]),
            ),
            (Some("gzip".to_owned()), true),
        );

        for (uri, expected_content_type, expected_cache_control) in [
            (
                "/assets/app-abcdef12.js",
                "text/javascript",
                "public, max-age=31536000, immutable",
            ),
            (
                "/assets/app-abcdef12.css",
                "text/css",
                "public, max-age=31536000, immutable",
            ),
            ("/", "text/html", "no-cache"),
            ("/index.html", "text/html", "no-cache"),
            ("/sessions/fixture-session", "text/html", "no-cache"),
        ] {
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
            let cache_control = response
                .headers()
                .get(CACHE_CONTROL)
                .map(|value| value.to_str().unwrap().to_owned());
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();

            assert_eq!(
                (
                    status,
                    content_type,
                    content_encoding,
                    cache_control,
                    body.starts_with(&[0x1f, 0x8b]),
                ),
                (
                    StatusCode::OK,
                    Some(expected_content_type.to_owned()),
                    Some("gzip".to_owned()),
                    Some(expected_cache_control.to_owned()),
                    true,
                ),
                "{uri}",
            );
        }
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
        let response = app(watch::channel(false).1)
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
