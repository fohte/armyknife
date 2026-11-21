use std::net::IpAddr;

use anyhow::Result;

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

    let Ok(mut parsed_url) = reqwest::Url::parse(url) else {
        return url.to_string();
    };
    parsed_url.set_path("/preview");
    parsed_url.to_string()
}

fn preview_mode_for_port(status: &serde_json::Value, port: u16) -> bool {
    session_for_port(status, port)
        .and_then(|session| session.get("args").and_then(serde_json::Value::as_array))
        .and_then(|args| args.first())
        .and_then(serde_json::Value::as_str)
        == Some("preview")
}

fn is_loopback_url(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::preview_root_url(
        Ok(serde_json::json!({
            "sessions": [{ "port": 43210, "args": ["preview", "sample.html"] }],
        })),
        "http://localhost:43210",
        "http://localhost:43210/preview",
    )]
    #[case::preview_root_url_with_query(
        Ok(serde_json::json!({
            "sessions": [{ "port": 43210, "args": ["preview", "sample.html"] }],
        })),
        "http://localhost:43210/?token=fixture",
        "http://localhost:43210/preview?token=fixture",
    )]
    #[case::non_preview_root_url(
        Ok(serde_json::json!({
            "sessions": [{ "port": 43210, "args": ["review", "sample.html"] }],
        })),
        "http://localhost:43210",
        "http://localhost:43210",
    )]
    #[case::preview_pathful_url(
        Ok(serde_json::json!({
            "sessions": [{ "port": 43210, "args": ["preview", "sample.html"] }],
        })),
        "http://localhost:43210/review",
        "http://localhost:43210/review",
    )]
    #[case::remote_root_url(
        Ok(serde_json::json!({
            "sessions": [{ "port": 43210, "args": ["preview", "sample.html"] }],
        })),
        "https://review.example.test:43210",
        "https://review.example.test:43210",
    )]
    #[case::preview_on_another_port(
        Ok(serde_json::json!({
            "sessions": [
                { "port": 43210, "args": ["review", "sample.html"] },
                { "port": 43211, "args": ["preview", "sample.html"] },
            ],
        })),
        "http://localhost:43210",
        "http://localhost:43210",
    )]
    #[case::status_unavailable(
        Err(anyhow::anyhow!("fixture status error")),
        "http://localhost:43210",
        "http://localhost:43210",
    )]
    fn registers_preview_path_only_for_local_preview_root_urls(
        #[case] status: Result<serde_json::Value>,
        #[case] url: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(
            prepare_registration_with_status(43210, url, || status).0,
            expected,
        );
    }
}
