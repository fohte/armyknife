use axum::body::Body;
use axum::http::header::CONTENT_TYPE;
use axum::http::{Response, StatusCode, Uri};
use axum::response::IntoResponse;
use include_dir::{Dir, include_dir};

pub(super) static DIST: Dir<'static> = include_dir!("$AGENT_WATCH_WEB_ASSETS");

const BUILD_INSTRUCTIONS: &str = "The Agent Watch Web UI has not been built. Run `cd agent-watch-web && pnpm install --frozen-lockfile && pnpm run build`, then rebuild armyknife.";

pub(super) fn serve_from(uri: &Uri, dist: &'static Dir<'static>) -> Response<Body> {
    if dist.get_file("index.html").is_none() {
        return (StatusCode::SERVICE_UNAVAILABLE, BUILD_INSTRUCTIONS).into_response();
    }

    let path = asset_path(uri);
    let (file, resolved_path) = match dist.get_file(path) {
        Some(file) => (Some(file), path),
        None => (dist.get_file("index.html"), "index.html"),
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

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::header::CONTENT_TYPE;
    use axum::http::{StatusCode, Uri};
    use include_dir::{Dir, include_dir};

    use super::{BUILD_INSTRUCTIONS, serve_from};

    static TEST_DIST: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/src/commands/agent/watch_web/test_assets");
    static EMPTY_DIST: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/src/commands/agent/watch_web/empty_test_assets");

    #[tokio::test]
    async fn unknown_client_routes_serve_the_index_file() {
        let response = serve_from(&"/sessions/example".parse::<Uri>().unwrap(), &TEST_DIST);
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|value| value.to_str().unwrap().to_owned());
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();

        assert_eq!(
            (status, content_type, body.to_vec()),
            (
                StatusCode::OK,
                Some("text/html".to_owned()),
                b"<main>fixture</main>\n".to_vec(),
            )
        );
    }

    #[tokio::test]
    async fn direct_assets_use_their_mime_type() {
        let response = serve_from(&"/assets/app.css".parse::<Uri>().unwrap(), &TEST_DIST);
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|value| value.to_str().unwrap().to_owned());
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();

        assert_eq!(
            (status, content_type, body.to_vec()),
            (
                StatusCode::OK,
                Some("text/css".to_owned()),
                include_bytes!("test_assets/assets/app.css").to_vec(),
            )
        );
    }

    #[tokio::test]
    async fn missing_assets_return_build_instructions() {
        let response = serve_from(&"/".parse::<Uri>().unwrap(), &EMPTY_DIST);
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|value| value.to_str().unwrap().to_owned());
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();

        assert_eq!(
            (status, content_type, body.to_vec()),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Some("text/plain; charset=utf-8".to_owned()),
                BUILD_INSTRUCTIONS.as_bytes().to_vec()
            )
        );
    }
}
