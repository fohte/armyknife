use serde_json::json;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, ResponseTemplate};

use super::GitHubMockServer;

impl GitHubMockServer {
    pub async fn graphql_pull_request_by_number_with_ci(&self, pull_request: serde_json::Value) {
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("pullRequest(number:"))
            .and(body_string_contains("statusCheckRollup { state }"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "repo0": {
                        "pr0": pull_request
                    }
                }
            })))
            .mount(&self.server)
            .await;
    }

    /// Mock numbered pull requests with partial data and per-alias errors.
    pub async fn graphql_pull_requests_by_number_with_ci(
        &self,
        pull_requests: Vec<Option<serde_json::Value>>,
        missing_indexes: &[usize],
    ) {
        let pull_request_data = pull_requests
            .into_iter()
            .enumerate()
            .map(|(index, pull_request)| {
                (
                    format!("pr{index}"),
                    pull_request.unwrap_or(serde_json::Value::Null),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let errors = missing_indexes
            .iter()
            .map(|index| {
                json!({
                    "message": "Could not resolve pull request",
                    "path": ["repo0", format!("pr{index}")],
                })
            })
            .collect::<Vec<_>>();

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("pullRequest(number:"))
            .and(body_string_contains("statusCheckRollup { state }"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "repo0": pull_request_data },
                "errors": errors,
            })))
            .mount(&self.server)
            .await;
    }
}
