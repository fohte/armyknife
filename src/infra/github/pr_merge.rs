//! Asynchronous pull request merge operations.

use std::time::{Duration, Instant};

use anyhow::Context;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{client::GitHubClient, error::api_error_from_response};

const ASYNC_MERGE_POLL_INTERVAL: Duration = Duration::from_secs(10);
const ASYNC_MERGE_PROGRESS_INTERVAL: Duration = Duration::from_secs(60);
// Stop before GitHub's 24-hour async result retention window expires.
const ASYNC_MERGE_MAX_WAIT: Duration = Duration::from_secs(23 * 60 * 60);

/// Final outcome of an asynchronous pull request merge request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsyncMergeResult {
    Merged { sha: Option<String> },
    Enqueued { message: Option<String> },
}

#[derive(Debug)]
enum AsyncMergeStatus {
    Pending { uuid: String },
    Merged { sha: Option<String> },
    Enqueued { message: Option<String> },
    Failed { message: Option<String> },
}

#[derive(Debug, Deserialize)]
struct AsyncMergeResponse {
    status: String,
    details: Option<AsyncMergeDetails>,
}

#[derive(Debug, Deserialize)]
struct AsyncMergeDetails {
    uuid: Option<String>,
    sha: Option<String>,
    message: Option<String>,
}

impl AsyncMergeResponse {
    fn into_status(self) -> anyhow::Result<AsyncMergeStatus> {
        let details = self.details.unwrap_or(AsyncMergeDetails {
            uuid: None,
            sha: None,
            message: None,
        });
        match self.status.as_str() {
            "pending" => Ok(AsyncMergeStatus::Pending {
                uuid: details
                    .uuid
                    .context("GitHub returned a pending merge without a UUID")?,
            }),
            "merged" => Ok(AsyncMergeStatus::Merged { sha: details.sha }),
            "enqueued" => Ok(AsyncMergeStatus::Enqueued {
                message: details.message,
            }),
            "failed" => Ok(AsyncMergeStatus::Failed {
                message: details.message,
            }),
            status => anyhow::bail!("GitHub returned unknown async merge status: {status}"),
        }
    }
}

impl GitHubClient {
    /// Request a squash merge and wait until GitHub reports a final result.
    pub async fn merge_pull_request_async(
        &self,
        owner: &str,
        repo: &str,
        pull_request: u64,
        head_sha: &str,
    ) -> anyhow::Result<AsyncMergeResult> {
        self.merge_pull_request_async_with_timing(
            owner,
            repo,
            pull_request,
            head_sha,
            ASYNC_MERGE_POLL_INTERVAL,
            ASYNC_MERGE_MAX_WAIT,
        )
        .await
    }

    async fn merge_pull_request_async_with_timing(
        &self,
        owner: &str,
        repo: &str,
        pull_request: u64,
        head_sha: &str,
        poll_interval: Duration,
        max_wait: Duration,
    ) -> anyhow::Result<AsyncMergeResult> {
        let route = format!("/repos/{owner}/{repo}/pulls/{pull_request}/merge-async");
        let response = self
            .http
            .put(self.url(&route))
            .json(&json!({ "sha": head_sha, "merge_method": "squash" }))
            .send()
            .await?;
        let status_code = response.status();
        let body: Value = response.json().await.unwrap_or_else(|_| json!({}));
        if !status_code.is_success()
            && (status_code != reqwest::StatusCode::CONFLICT || body.get("status").is_none())
        {
            return Err(api_error_from_response(status_code.as_u16(), &body).into());
        }
        let mut status: AsyncMergeStatus = serde_json::from_value::<AsyncMergeResponse>(body)
            .context("Failed to parse async merge response")?
            .into_status()?;
        let started_at = Instant::now();
        let mut last_progress_at: Option<Instant> = None;

        loop {
            match status {
                AsyncMergeStatus::Pending { uuid } => {
                    if started_at.elapsed() >= max_wait {
                        anyhow::bail!(
                            "Timed out waiting for async merge request {uuid} for PR #{pull_request}."
                        );
                    }
                    if last_progress_at
                        .is_none_or(|last| last.elapsed() >= ASYNC_MERGE_PROGRESS_INTERVAL)
                    {
                        eprintln!(
                            "PR #{pull_request} merge is pending; waiting for GitHub (request {uuid})."
                        );
                        last_progress_at = Some(Instant::now());
                    }
                    tokio::time::sleep(poll_interval).await;
                    let result_route = format!("{route}/{uuid}");
                    let response: AsyncMergeResponse = self.rest_get(&result_route).await?;
                    status = response.into_status()?;
                }
                AsyncMergeStatus::Merged { sha } => {
                    return Ok(AsyncMergeResult::Merged { sha });
                }
                AsyncMergeStatus::Enqueued { message } => {
                    return Ok(AsyncMergeResult::Enqueued { message });
                }
                AsyncMergeStatus::Failed { message } => {
                    let message =
                        message.unwrap_or_else(|| "No failure details were returned.".into());
                    anyhow::bail!("Asynchronous merge failed: {message}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;

    #[rstest::rstest]
    #[tokio::test]
    async fn merge_poll_sends_the_head_sha_and_returns_the_final_result() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/example-org/example-repo/pulls/42/merge-async"))
            .and(body_json(
                json!({ "sha": "head-sha", "merge_method": "squash" }),
            ))
            .respond_with(ResponseTemplate::new(202).set_body_json(json!({
                "status": "pending",
                "details": { "uuid": "request-id" }
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/repos/example-org/example-repo/pulls/42/merge-async/request-id",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "merged",
                "details": { "sha": "merge-sha" }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = GitHubClient::with_base_url(&server.uri(), "test-token").unwrap();

        let result = client
            .merge_pull_request_async_with_timing(
                "example-org",
                "example-repo",
                42,
                "head-sha",
                Duration::ZERO,
                Duration::from_secs(60),
            )
            .await;

        assert_eq!(
            result.map_err(|error| error.to_string()),
            Ok(AsyncMergeResult::Merged {
                sha: Some("merge-sha".into())
            }),
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn merge_poll_reuses_a_pending_request_after_conflict_and_returns_enqueued() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/example-org/example-repo/pulls/42/merge-async"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "status": "pending",
                "details": { "uuid": "existing-request" }
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/repos/example-org/example-repo/pulls/42/merge-async/existing-request",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "enqueued",
                "details": { "message": "Waiting for a queue slot." }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = GitHubClient::with_base_url(&server.uri(), "test-token").unwrap();

        let result = client
            .merge_pull_request_async_with_timing(
                "example-org",
                "example-repo",
                42,
                "head-sha",
                Duration::ZERO,
                Duration::from_secs(60),
            )
            .await;

        assert_eq!(
            result.map_err(|error| error.to_string()),
            Ok(AsyncMergeResult::Enqueued {
                message: Some("Waiting for a queue slot.".into()),
            }),
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn merge_poll_returns_the_github_message_for_a_conflict_without_a_request() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/example-org/example-repo/pulls/42/merge-async"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "message": "The head SHA does not match."
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = GitHubClient::with_base_url(&server.uri(), "test-token").unwrap();

        let result = client
            .merge_pull_request_async_with_timing(
                "example-org",
                "example-repo",
                42,
                "head-sha",
                Duration::ZERO,
                Duration::from_secs(60),
            )
            .await;

        assert_eq!(
            result.map_err(|error| error.to_string()),
            Err("GitHub API error: The head SHA does not match. (HTTP 409)".into()),
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn merge_poll_times_out_with_the_pending_request_uuid() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/example-org/example-repo/pulls/42/merge-async"))
            .respond_with(ResponseTemplate::new(202).set_body_json(json!({
                "status": "pending",
                "details": { "uuid": "stalled-request" }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = GitHubClient::with_base_url(&server.uri(), "test-token").unwrap();

        let result = client
            .merge_pull_request_async_with_timing(
                "example-org",
                "example-repo",
                42,
                "head-sha",
                Duration::ZERO,
                Duration::ZERO,
            )
            .await;

        assert_eq!(
            result.map_err(|error| error.to_string()),
            Err("Timed out waiting for async merge request stalled-request for PR #42.".into()),
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn merge_poll_surfaces_github_failure_messages() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/example-org/example-repo/pulls/42/merge-async"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "failed",
                "details": { "message": "Required checks are pending." }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = GitHubClient::with_base_url(&server.uri(), "test-token").unwrap();

        let result = client
            .merge_pull_request_async_with_timing(
                "example-org",
                "example-repo",
                42,
                "head-sha",
                Duration::ZERO,
                Duration::from_secs(60),
            )
            .await;

        assert_eq!(
            result.map_err(|error| error.to_string()),
            Err("Asynchronous merge failed: Required checks are pending.".into()),
        );
    }
}
