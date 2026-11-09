use axum::extract::Request;
use axum::http::header::{HOST, ORIGIN};
use axum::http::uri::{Authority, Uri};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

pub(super) async fn validate_request(request: Request, next: Next) -> Response {
    if request_is_allowed(request.method(), request.headers()) {
        next.run(request).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}

fn request_is_allowed(method: &Method, headers: &axum::http::HeaderMap) -> bool {
    let Some(host) = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<Authority>().ok())
    else {
        return false;
    };

    if host.as_str().contains('@') || !allowed_host(host.host()) {
        return false;
    }

    if method == Method::GET || method == Method::HEAD {
        return true;
    }

    headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<Uri>().ok())
        .filter(|origin| {
            matches!(origin.scheme_str(), Some("http" | "https"))
                && origin
                    .path_and_query()
                    .is_none_or(|path| path.as_str() == "/")
        })
        .and_then(|origin| origin.authority().cloned())
        .is_some_and(|origin| {
            !origin.as_str().contains('@') && origin.as_str().eq_ignore_ascii_case(host.as_str())
        })
}

fn allowed_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "localhost" || host == "127.0.0.1" || (host.ends_with(".ts.net") && host.len() > 7)
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};
    use rstest::rstest;

    use super::request_is_allowed;

    #[rstest]
    #[case::localhost("localhost:4317", true)]
    #[case::loopback("127.0.0.1:4317", true)]
    #[case::tailscale_host("device.example.ts.net", true)]
    #[case::userinfo_host("user@localhost", false)]
    #[case::tailscale_suffix_is_not_a_host("example.ts.net.attacker.invalid", false)]
    #[case::remote_ip("192.0.2.10:4317", false)]
    #[case::missing_host("", false)]
    fn only_loopback_and_tailscale_hosts_are_allowed(#[case] host: &str, #[case] expected: bool) {
        let mut headers = HeaderMap::new();
        if !host.is_empty() {
            headers.insert("host", HeaderValue::from_str(host).unwrap());
        }

        assert_eq!(
            request_is_allowed(&axum::http::Method::GET, &headers),
            expected
        );
    }

    #[rstest]
    #[case::matching_origin("localhost:4317", "http://localhost:4317", true)]
    #[case::matching_tailscale_origin(
        "device.example.ts.net",
        "https://device.example.ts.net",
        true
    )]
    #[case::origin_with_userinfo(
        "device.example.ts.net",
        "https://user@device.example.ts.net",
        false
    )]
    #[case::different_host("localhost:4317", "http://attacker.invalid:4317", false)]
    #[case::different_port("localhost:4317", "http://localhost:4318", false)]
    #[case::origin_with_path("localhost:4317", "http://localhost:4317/path", false)]
    #[case::relative_origin("localhost:4317", "//localhost:4317", false)]
    #[case::missing_origin("localhost:4317", "", false)]
    fn writes_require_an_origin_matching_host(
        #[case] host: &str,
        #[case] origin: &str,
        #[case] expected: bool,
    ) {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_str(host).unwrap());
        if !origin.is_empty() {
            headers.insert("origin", HeaderValue::from_str(origin).unwrap());
        }

        assert_eq!(
            request_is_allowed(&axum::http::Method::POST, &headers),
            expected
        );
    }

    #[rstest]
    #[case::get(axum::http::Method::GET, true)]
    #[case::head(axum::http::Method::HEAD, true)]
    fn reads_do_not_require_an_origin(#[case] method: axum::http::Method, #[case] expected: bool) {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("localhost"));

        assert_eq!(request_is_allowed(&method, &headers), expected);
    }
}
