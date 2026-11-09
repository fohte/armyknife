mod actions;
mod assets;
mod security;

use std::net::{Ipv4Addr, SocketAddr};

use anyhow::Result;
use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::any;
use clap::Args;
use include_dir::Dir;
use tokio::net::TcpListener;

use super::browser;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WatchWebArgs {}

const LISTEN_PORT: u16 = 4317;

pub async fn run(_args: &WatchWebArgs) -> Result<()> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, LISTEN_PORT));
    let listener = TcpListener::bind(address).await?;
    let url = format!("http://{address}");
    println!("Agent Watch Web UI: {url}");
    if let Err(error) = browser::open_url(&url) {
        eprintln!("Could not open a browser automatically: {error}");
    }

    axum::serve(listener, app()).await?;
    Ok(())
}

fn app() -> Router {
    app_with_assets(&assets::DIST)
}

fn app_with_assets(dist: &'static Dir<'static>) -> Router {
    let api = Router::new()
        .merge(actions::router())
        .fallback(api_not_found);

    Router::new()
        .nest("/api", api)
        .route("/api/", any(api_not_found))
        .fallback(move |request: axum::extract::Request| async move {
            assets::serve_from(request.uri(), dist)
        })
        .layer(axum::middleware::from_fn(security::validate_request))
}

async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "API route not found")
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::header::{CONTENT_TYPE, HOST, ORIGIN};
    use axum::http::{Request, Response, StatusCode};
    use include_dir::{Dir, include_dir};
    use rstest::rstest;
    use tower::ServiceExt;

    use super::{app, app_with_assets};

    static TEST_DIST: Dir<'static> =
        include_dir!("$CARGO_MANIFEST_DIR/src/commands/agent/watch_web/test_assets");

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
        "<main>fixture</main>\n"
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
        let response = app()
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

    #[tokio::test]
    async fn matching_origin_reaches_the_action_route() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/sessions/not-found/answer")
                    .header(HOST, "localhost")
                    .header(ORIGIN, "http://localhost")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
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
