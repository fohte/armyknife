use anyhow::Result;

use super::super::url::local_crit_ip;
use super::status::{crit_status, session_for_port};

pub(super) fn prepare_registration(port: u16, url: &str) -> (String, Option<serde_json::Value>) {
    prepare_registration_with_status(port, url, crit_status)
}

fn prepare_registration_with_status(
    port: u16,
    url: &str,
    status: impl FnOnce() -> Result<serde_json::Value>,
) -> (String, Option<serde_json::Value>) {
    let Ok(parsed_url) = reqwest::Url::parse(url) else {
        return (url.to_string(), None);
    };
    if !matches!(parsed_url.path(), "" | "/") || !is_loopback_url(&parsed_url) {
        return (url.to_string(), None);
    }

    match status() {
        Ok(status) => {
            let registered_url = registered_url_with_status(port, url, &status);
            (registered_url, Some(status))
        }
        Err(error) => {
            tracing::warn!(
                event = "agent.crit.registration.status_failed",
                port,
                error = %error,
            );
            (url.to_string(), None)
        }
    }
}

fn registered_url_with_status(port: u16, url: &str, status: &serde_json::Value) -> String {
    if !preview_mode_for_port(status, port) {
        return url.to_string();
    }

    let Ok(parsed_url) = reqwest::Url::parse(url) else {
        return url.to_string();
    };
    if !matches!(parsed_url.path(), "" | "/") {
        return url.to_string();
    }
    // URL serialization drops explicit default ports required by crit's monitor parser.
    let authority_start = url
        .find("://")
        .map_or(url.len(), |scheme_end| scheme_end + 3);
    let path_start = url[authority_start..]
        .find(['/', '?', '#'])
        .map(|suffix_start| authority_start + suffix_start)
        .unwrap_or(url.len());
    let suffix_start = if url[path_start..].starts_with('/') {
        path_start + 1
    } else {
        path_start
    };
    format!("{}/preview{}", &url[..path_start], &url[suffix_start..])
}

fn preview_mode_for_port(status: &serde_json::Value, port: u16) -> bool {
    session_for_port(status, port)
        .and_then(|session| session.get("args").and_then(serde_json::Value::as_array))
        .and_then(|args| args.first())
        .and_then(serde_json::Value::as_str)
        == Some("preview")
}

fn is_loopback_url(url: &reqwest::Url) -> bool {
    url.host_str().and_then(local_crit_ip).is_some()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::preview_root_url(
        43210,
        Ok(serde_json::json!({
            "sessions": [{ "port": 43210, "args": ["preview", "sample.html"] }],
        })),
        "http://localhost:43210",
        true,
        "http://localhost:43210/preview",
    )]
    #[case::preview_root_url_with_query(
        43210,
        Ok(serde_json::json!({
            "sessions": [{ "port": 43210, "args": ["preview", "sample.html"] }],
        })),
        "http://localhost:43210/?token=fixture",
        true,
        "http://localhost:43210/preview?token=fixture",
    )]
    #[case::non_preview_root_url(
        43210,
        Ok(serde_json::json!({
            "sessions": [{ "port": 43210, "args": ["review", "sample.html"] }],
        })),
        "http://localhost:43210",
        true,
        "http://localhost:43210",
    )]
    #[case::preview_pathful_url(
        43210,
        Ok(serde_json::json!({
            "sessions": [{ "port": 43210, "args": ["preview", "sample.html"] }],
        })),
        "http://localhost:43210/review",
        false,
        "http://localhost:43210/review",
    )]
    #[case::remote_root_url(
        43210,
        Ok(serde_json::json!({
            "sessions": [{ "port": 43210, "args": ["preview", "sample.html"] }],
        })),
        "https://review.example.test:43210",
        false,
        "https://review.example.test:43210",
    )]
    #[case::preview_on_another_port(
        43210,
        Ok(serde_json::json!({
            "sessions": [
                { "port": 43210, "args": ["review", "sample.html"] },
                { "port": 43211, "args": ["preview", "sample.html"] },
            ],
        })),
        "http://localhost:43210",
        true,
        "http://localhost:43210",
    )]
    #[case::preview_default_http_port(
        80,
        Ok(serde_json::json!({
            "sessions": [{ "port": 80, "args": ["preview", "sample.html"] }],
        })),
        "http://localhost:80",
        true,
        "http://localhost:80/preview",
    )]
    #[case::preview_ipv6_loopback_url(
        43210,
        Ok(serde_json::json!({
            "sessions": [{ "port": 43210, "args": ["preview", "sample.html"] }],
        })),
        "http://[::1]:43210",
        true,
        "http://[::1]:43210/preview",
    )]
    #[case::preview_localhost_subdomain(
        43210,
        Ok(serde_json::json!({
            "sessions": [{ "port": 43210, "args": ["preview", "sample.html"] }],
        })),
        "http://design.localhost:43210",
        true,
        "http://design.localhost:43210/preview",
    )]
    #[case::status_unavailable(
        43210,
        Err(anyhow::anyhow!("fixture status error")),
        "http://localhost:43210",
        false,
        "http://localhost:43210",
    )]
    fn registers_preview_path_only_for_local_preview_root_urls(
        #[case] port: u16,
        #[case] status: Result<serde_json::Value>,
        #[case] url: &str,
        #[case] expect_status_snapshot: bool,
        #[case] expected: &str,
    ) {
        let expected_status = if expect_status_snapshot {
            status.as_ref().ok().cloned()
        } else {
            None
        };
        assert_eq!(
            prepare_registration_with_status(port, url, || status),
            (expected.to_string(), expected_status),
        );
    }
}
