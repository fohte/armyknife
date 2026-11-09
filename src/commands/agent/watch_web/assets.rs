use axum::body::Body;
use axum::extract::Request;
use axum::http::header::CONTENT_TYPE;
use axum::http::{Response, StatusCode, Uri};
use axum::response::IntoResponse;
use include_dir::{Dir, include_dir};

static DIST: Dir<'_> = include_dir!("$AGENT_WATCH_WEB_ASSETS");

const BUILD_INSTRUCTIONS: &str = "The Agent Watch Web UI has not been built. Run `cd agent-watch-web && pnpm install --frozen-lockfile && pnpm run build`, then rebuild armyknife.";

pub(super) async fn serve(request: Request) -> Response<Body> {
    if DIST.get_file("index.html").is_none() {
        return (StatusCode::SERVICE_UNAVAILABLE, BUILD_INSTRUCTIONS).into_response();
    }

    let uri = request.uri().clone();
    let path = asset_path(&uri);
    let (file, resolved_path) = match DIST.get_file(path) {
        Some(file) => (Some(file), path),
        None => (DIST.get_file("index.html"), "index.html"),
    };
    let Some(file) = file else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let content_type = mime_guess::from_path(resolved_path)
        .first_or_octet_stream()
        .to_string();
    match Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, content_type)
        .body(Body::from(file.contents()))
    {
        Ok(response) => response,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn asset_path(uri: &Uri) -> &str {
    let path = uri.path().trim_start_matches('/');
    if path.is_empty() { "index.html" } else { path }
}
