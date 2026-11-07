//! Asynchronous pull request merge operations.

use std::time::{Duration, Instant};

use anyhow::Context;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{client::GitHubClient, error::api_error_from_response};

const ASYNC_MERGE_POLL_INTERVAL: Duration = Duration::from_secs(10);
const ASYNC_MERGE_PROGRESS_INTERVAL: Duration = Duration::from_secs(60);
const MERGE_QUEUE_QUERY: &str = "query($owner: String!, $name: String!, $branch: String!) { repository(owner: $owner, name: $name) { mergeQueue(branch: $branch) { id } } }";
// Keep an interactive command from waiting too long on an uncancelable pending request.
const ASYNC_MERGE_MAX_WAIT: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AsyncMergeFallback {
    #[error("Timed out waiting for async merge request {request_uuid} for PR #{pull_request}.")]
    TimedOut {
        request_uuid: String,
        pull_request: u64,
    },
    #[error(
        "Existing async merge request {request_uuid} for PR #{pull_request} targets head SHA {pending_head_sha}, not {current_head_sha}."
    )]
    HeadMismatch {
        request_uuid: String,
        pull_request: u64,
        pending_head_sha: String,
        current_head_sha: String,
    },
    #[error(
        "Existing async merge request {request_uuid} for PR #{pull_request} did not report its expected head SHA."
    )]
    HeadUnavailable {
        request_uuid: String,
        pull_request: u64,
    },
    #[error(
        "GitHub returned async merge request {response_uuid} when checking existing request {request_uuid} for PR #{pull_request}."
    )]
    RequestUuidMismatch {
        request_uuid: String,
        response_uuid: String,
        pull_request: u64,
    },
}

impl AsyncMergeFallback {
    pub fn request_uuid(&self) -> &str {
        match self {
            Self::TimedOut { request_uuid, .. }
            | Self::HeadMismatch { request_uuid, .. }
            | Self::HeadUnavailable { request_uuid, .. }
            | Self::RequestUuidMismatch { request_uuid, .. } => request_uuid,
        }
    }
}

/// Final outcome of an asynchronous pull request merge request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeResult {
    Merged { sha: Option<String> },
    Enqueued { message: Option<String> },
}

pub type AsyncMergeResult = MergeResult;

#[derive(Debug)]
enum AsyncMergeStatus {
    Pending {
        uuid: String,
        expected_head_sha: Option<String>,
    },
    Merged {
        sha: Option<String>,
    },
    Enqueued {
        message: Option<String>,
    },
    Failed {
        message: Option<String>,
    },
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
    expected_head_sha: Option<String>,
}

impl AsyncMergeResponse {
    fn into_status(self) -> anyhow::Result<AsyncMergeStatus> {
        let details = self.details.unwrap_or(AsyncMergeDetails {
            uuid: None,
            sha: None,
            message: None,
            expected_head_sha: None,
        });
        match self.status.as_str() {
            "pending" => Ok(AsyncMergeStatus::Pending {
                uuid: details
                    .uuid
                    .context("GitHub returned a pending merge without a UUID")?,
                expected_head_sha: details.expected_head_sha,
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
    /// Request a squash merge and wait up to five minutes for GitHub's final result.
    /// A timeout exposes the pending request UUID for diagnosis and recovery.
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
        if status_code == reqwest::StatusCode::CONFLICT {
            status = self
                .verify_existing_request(&route, pull_request, head_sha, status)
                .await?;
        }
        let started_at = Instant::now();
        let mut last_progress_at: Option<Instant> = None;

        loop {
            match status {
                AsyncMergeStatus::Pending { uuid, .. } => {
                    if started_at.elapsed() >= max_wait {
                        return Err(AsyncMergeFallback::TimedOut {
                            request_uuid: uuid,
                            pull_request,
                        }
                        .into());
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

    async fn verify_existing_request(
        &self,
        route: &str,
        pull_request: u64,
        head_sha: &str,
        status: AsyncMergeStatus,
    ) -> anyhow::Result<AsyncMergeStatus> {
        let uuid = match &status {
            AsyncMergeStatus::Pending { uuid, .. } => uuid.clone(),
            _ => anyhow::bail!(
                "GitHub returned a 409 response without a pending merge request for PR #{pull_request}."
            ),
        };

        if let AsyncMergeStatus::Pending {
            expected_head_sha: Some(expected_head_sha),
            ..
        } = &status
        {
            if expected_head_sha != head_sha {
                return Err(AsyncMergeFallback::HeadMismatch {
                    request_uuid: uuid,
                    pull_request,
                    pending_head_sha: expected_head_sha.clone(),
                    current_head_sha: head_sha.to_string(),
                }
                .into());
            }
            return Ok(status);
        }

        let result_route = format!("{route}/{uuid}");
        let pending_status: AsyncMergeStatus = self
            .rest_get::<AsyncMergeResponse>(&result_route)
            .await?
            .into_status()?;
        let actual_head_sha = match &pending_status {
            AsyncMergeStatus::Pending {
                uuid: response_uuid,
                expected_head_sha: Some(expected_head_sha),
            } if response_uuid == &uuid => expected_head_sha.clone(),
            AsyncMergeStatus::Pending {
                uuid: response_uuid,
                ..
            } if response_uuid != &uuid => {
                return Err(AsyncMergeFallback::RequestUuidMismatch {
                    request_uuid: uuid,
                    response_uuid: response_uuid.clone(),
                    pull_request,
                }
                .into());
            }
            AsyncMergeStatus::Pending {
                expected_head_sha: None,
                ..
            } => {
                return Err(AsyncMergeFallback::HeadUnavailable {
                    request_uuid: uuid,
                    pull_request,
                }
                .into());
            }
            _ => anyhow::bail!(
                "Existing async merge request {uuid} for PR #{pull_request} completed before its expected head SHA could be verified."
            ),
        };
        if actual_head_sha != head_sha {
            return Err(AsyncMergeFallback::HeadMismatch {
                request_uuid: uuid,
                pull_request,
                pending_head_sha: actual_head_sha,
                current_head_sha: head_sha.to_string(),
            }
            .into());
        }
        Ok(pending_status)
    }

    pub async fn merge_pull_request(
        &self,
        owner: &str,
        repo: &str,
        pull_request: u64,
        head_sha: &str,
    ) -> anyhow::Result<MergeResult> {
        let route = format!("/repos/{owner}/{repo}/pulls/{pull_request}/merge");
        let response = self
            .http
            .put(self.url(&route))
            .json(&json!({ "sha": head_sha, "merge_method": "squash" }))
            .send()
            .await?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or_else(|_| json!({}));
        if !status.is_success() {
            return Err(api_error_from_response(status.as_u16(), &body).into());
        }
        let response: SyncMergeResponse =
            serde_json::from_value(body).context("Failed to parse synchronous merge response")?;
        if !response.merged {
            anyhow::bail!(
                "Synchronous merge failed: {}",
                response
                    .message
                    .unwrap_or_else(|| "No details were returned.".into())
            );
        }
        Ok(MergeResult::Merged { sha: response.sha })
    }

    pub async fn branch_requires_merge_queue(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> anyhow::Result<bool> {
        let response: MergeQueueQueryResponse = self
            .graphql(
                MERGE_QUEUE_QUERY,
                json!({ "owner": owner, "name": repo, "branch": branch }),
            )
            .await?;
        let repository = response
            .repository
            .context("GitHub did not return the repository when checking its merge queue")?;
        Ok(repository.merge_queue.is_some())
    }
}

#[derive(Debug, Deserialize)]
struct SyncMergeResponse {
    merged: bool,
    sha: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeQueueQueryResponse {
    repository: Option<MergeQueueRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeQueueRepository {
    merge_queue: Option<serde::de::IgnoredAny>,
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
    async fn sync_merge_sends_the_expected_head_and_uses_squash() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/example-org/example-repo/pulls/42/merge"))
            .and(body_json(
                json!({ "sha": "head-sha", "merge_method": "squash" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "merged": true,
                "sha": "merge-sha",
                "message": "Pull Request successfully merged"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = GitHubClient::with_base_url(&server.uri(), "test-token").unwrap();

        let result = client
            .merge_pull_request("example-org", "example-repo", 42, "head-sha")
            .await;

        assert_eq!(
            result.map_err(|error| error.to_string()),
            Ok(AsyncMergeResult::Merged {
                sha: Some("merge-sha".into())
            }),
        );
    }

    #[rstest::rstest]
    #[case::http_error(
        405,
        json!({ "message": "Pull Request is not mergeable" }),
        "GitHub API error: Pull Request is not mergeable (HTTP 405)"
    )]
    #[case::not_merged(
        200,
        json!({ "merged": false, "message": "Head branch was modified" }),
        "Synchronous merge failed: Head branch was modified"
    )]
    #[tokio::test]
    async fn sync_merge_surfaces_github_failures(
        #[case] status: u16,
        #[case] body: Value,
        #[case] expected_error: &str,
    ) {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/example-org/example-repo/pulls/42/merge"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let client = GitHubClient::with_base_url(&server.uri(), "test-token").unwrap();

        let result = client
            .merge_pull_request("example-org", "example-repo", 42, "head-sha")
            .await;

        assert_eq!(
            result.map_err(|error| error.to_string()),
            Err(expected_error.to_string()),
        );
    }

    #[rstest::rstest]
    #[case::merge_queue_required(true, true)]
    #[case::merge_queue_absent(false, false)]
    #[tokio::test]
    async fn branch_rules_report_whether_merge_queue_is_required(
        #[case] merge_queue_exists: bool,
        #[case] expected: bool,
    ) {
        let server = MockServer::start().await;
        let merge_queue = if merge_queue_exists {
            json!({ "id": "queue-id" })
        } else {
            Value::Null
        };
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_json(json!({
                "query": MERGE_QUEUE_QUERY,
                "variables": {
                    "owner": "example-org",
                    "name": "example-repo",
                    "branch": "main"
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "repository": {
                        "mergeQueue": merge_queue
                    }
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = GitHubClient::with_base_url(&server.uri(), "test-token").unwrap();

        let result = client
            .branch_requires_merge_queue("example-org", "example-repo", "main")
            .await;

        assert_eq!(result.map_err(|error| error.to_string()), Ok(expected));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn merge_poll_reuses_a_pending_request_after_conflict_and_returns_enqueued() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/example-org/example-repo/pulls/42/merge-async"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "status": "pending",
                "details": {
                    "uuid": "existing-request",
                    "expected_head_sha": "head-sha"
                }
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
    async fn merge_poll_rejects_a_conflicting_request_for_a_different_head_sha() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/example-org/example-repo/pulls/42/merge-async"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "status": "pending",
                "details": {
                    "uuid": "existing-request",
                    "expected_head_sha": "older-head-sha"
                }
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
            result.map_err(|error| error.downcast::<AsyncMergeFallback>().ok()),
            Err(Some(AsyncMergeFallback::HeadMismatch {
                request_uuid: "existing-request".into(),
                pull_request: 42,
                pending_head_sha: "older-head-sha".into(),
                current_head_sha: "head-sha".into(),
            })),
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn merge_poll_reuses_a_same_head_request_when_conflict_omits_the_head_sha() {
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
                "status": "pending",
                "details": {
                    "uuid": "existing-request",
                    "expected_head_sha": "head-sha"
                }
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
            result.map_err(|error| error.downcast::<AsyncMergeFallback>().ok()),
            Err(Some(AsyncMergeFallback::TimedOut {
                request_uuid: "existing-request".into(),
                pull_request: 42,
            })),
        );
    }

    #[rstest::rstest]
    #[case::head_unavailable(
        json!({
            "status": "pending",
            "details": { "uuid": "existing-request" }
        }),
        AsyncMergeFallback::HeadUnavailable {
            request_uuid: "existing-request".into(),
            pull_request: 42,
        }
    )]
    #[case::request_uuid_mismatch(
        json!({
            "status": "pending",
            "details": {
                "uuid": "different-request",
                "expected_head_sha": "head-sha"
            }
        }),
        AsyncMergeFallback::RequestUuidMismatch {
            request_uuid: "existing-request".into(),
            response_uuid: "different-request".into(),
            pull_request: 42,
        }
    )]
    #[tokio::test]
    async fn merge_poll_returns_typed_fallback_when_existing_request_cannot_be_verified(
        #[case] status_body: Value,
        #[case] expected: AsyncMergeFallback,
    ) {
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
            .respond_with(ResponseTemplate::new(200).set_body_json(status_body))
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
            result.map_err(|error| error.downcast::<AsyncMergeFallback>().ok()),
            Err(Some(expected)),
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
