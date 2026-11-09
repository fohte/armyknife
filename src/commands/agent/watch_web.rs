mod assets;
mod security;

use std::net::{Ipv4Addr, SocketAddr};

use anyhow::Result;
use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use clap::Args;
use tokio::net::TcpListener;

use crate::shared::command;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WatchWebArgs {}

const LISTEN_PORT: u16 = 4317;

pub async fn run(_args: &WatchWebArgs) -> Result<()> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, LISTEN_PORT));
    let listener = TcpListener::bind(address).await?;
    let url = format!("http://{address}");
    println!("Agent Watch Web UI: {url}");
    open_browser(&url);

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

fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };

    if let Err(error) = command::new(opener).arg(url).spawn() {
        eprintln!("Could not open a browser automatically: {error}");
    }
}
