use axum::extract::Request;
use axum::http::header::{HOST, ORIGIN, UPGRADE};
use axum::http::uri::{Authority, Uri};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

const ALLOWED_HOSTS_ENV: &str = "ARMYKNIFE_WATCH_WEB_ALLOWED_HOSTS";

pub(super) async fn validate_request(request: Request, next: Next) -> Response {
    let allowed_hosts = std::env::var(ALLOWED_HOSTS_ENV).unwrap_or_default();
    if request_is_allowed(request.method(), request.headers(), &allowed_hosts) {
        next.run(request).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}

fn request_is_allowed(method: &Method, headers: &HeaderMap, allowed_hosts: &str) -> bool {
    let Some(host) = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<Authority>().ok())
    else {
        return false;
    };

    if host.as_str().contains('@') || !allowed_host(host.host(), allowed_hosts) {
        return false;
    }

    if (method == Method::GET || method == Method::HEAD) && !is_websocket_upgrade(headers) {
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

pub(super) fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
}

fn allowed_host(host: &str, allowed_hosts: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }

    let is_valid_domain = host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });

    is_valid_domain
        && allowed_hosts
            .split(',')
            .any(|allowed_host| allowed_host.trim().eq_ignore_ascii_case(host))
}

#[cfg(test)]
mod tests {
    use axum::http::header::{ORIGIN, UPGRADE};
    use axum::http::{HeaderMap, HeaderValue, Method};
    use rstest::rstest;

    use super::request_is_allowed;

    #[rstest]
    #[case::localhost("localhost:4317", "", true)]
    #[case::loopback("127.0.0.1:4317", "", true)]
    #[case::unconfigured_domain("watch.example.test", "", false)]
    #[case::configured_domain("watch.example.test", "watch.example.test", true)]
    #[case::wrong_configured_domain("watch.evil.test", "watch.example.test", false)]
    #[case::userinfo_host("user@localhost", "", false)]
    #[case::domain_with_empty_label("watch..example.test", "watch..example.test", false)]
    #[case::domain_with_leading_hyphen("-watch.example.test", "-watch.example.test", false)]
    #[case::remote_ip("192.0.2.10:4317", "192.0.2.10", false)]
    #[case::missing_host("", "", false)]
    fn loopback_and_configured_hosts_are_allowed(
        #[case] host: &str,
        #[case] allowed_hosts: &str,
        #[case] expected: bool,
    ) {
        let mut headers = HeaderMap::new();
        if !host.is_empty() {
            headers.insert("host", HeaderValue::from_str(host).unwrap());
        }

        assert_eq!(
            request_is_allowed(&axum::http::Method::GET, &headers, allowed_hosts),
            expected
        );
    }

    #[rstest]
    #[case::matching_origin("localhost:4317", "http://localhost:4317", "", true)]
    #[case::matching_external_origin(
        "watch.example.test",
        "https://watch.example.test",
        "watch.example.test",
        true
    )]
    #[case::origin_with_userinfo(
        "watch.example.test",
        "https://user@watch.example.test",
        "watch.example.test",
        false
    )]
    #[case::different_host("localhost:4317", "http://attacker.invalid:4317", "", false)]
    #[case::different_port("localhost:4317", "http://localhost:4320", "", false)]
    #[case::origin_with_path("localhost:4317", "http://localhost:4317/path", "", false)]
    #[case::relative_origin("localhost:4317", "//localhost:4317", "", false)]
    #[case::null_origin("localhost:4317", "null", "", false)]
    #[case::missing_origin("localhost:4317", "", "", false)]
    fn writes_require_an_origin_matching_host(
        #[case] host: &str,
        #[case] origin: &str,
        #[case] allowed_hosts: &str,
        #[case] expected: bool,
    ) {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_str(host).unwrap());
        if !origin.is_empty() {
            headers.insert("origin", HeaderValue::from_str(origin).unwrap());
        }

        assert_eq!(
            request_is_allowed(&axum::http::Method::POST, &headers, allowed_hosts),
            expected
        );
    }

    #[rstest]
    #[case::missing_origin(None, false)]
    #[case::different_port(Some("http://localhost:4320"), false)]
    #[case::matching_origin(Some("http://localhost:4317"), true)]
    fn websocket_upgrades_require_an_origin_matching_host(
        #[case] origin: Option<&str>,
        #[case] expected: bool,
    ) {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("localhost:4317"));
        headers.insert(UPGRADE, HeaderValue::from_static("websocket"));
        if let Some(origin) = origin {
            headers.insert(ORIGIN, HeaderValue::from_str(origin).unwrap());
        }

        assert_eq!(request_is_allowed(&Method::GET, &headers, ""), expected);
    }

    #[rstest]
    #[case::get(axum::http::Method::GET, true)]
    #[case::head(axum::http::Method::HEAD, true)]
    fn reads_do_not_require_an_origin(#[case] method: axum::http::Method, #[case] expected: bool) {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("localhost"));

        assert_eq!(request_is_allowed(&method, &headers, ""), expected);
    }
}
