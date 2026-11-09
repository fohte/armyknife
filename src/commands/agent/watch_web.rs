mod assets;
mod security;

use std::net::{Ipv4Addr, SocketAddr};

use anyhow::Result;
use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use clap::Args;
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
    let api = Router::new().fallback(api_not_found);

    Router::new()
        .nest("/api", api)
        .fallback(assets::serve)
        .layer(axum::middleware::from_fn(security::validate_request))
}

async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "API route not found")
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::header::{CONTENT_TYPE, HOST};
    use axum::http::{Request, Response, StatusCode};
    use tower::ServiceExt;

    use super::app;

    #[tokio::test]
    async fn unknown_api_routes_return_not_found_instead_of_the_spa() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/api/unimplemented")
                    .header(HOST, "localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response).await,
            (
                StatusCode::NOT_FOUND,
                Some("text/plain; charset=utf-8".to_owned()),
                "API route not found".to_owned(),
            )
        );
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
