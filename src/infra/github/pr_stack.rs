//! Pull request stack operations.

use serde::Deserialize;
use serde_json::json;

use super::client::GitHubClient;
use super::error::{Result, api_error_from_response};

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
    fn into_status(self) -> Result<AsyncMergeStatus> {
        let details = self.details.unwrap_or(AsyncMergeDetails {
            uuid: None,
            sha: None,
            message: None,
        });
        match self.status.as_str() {
            "pending" => Ok(AsyncMergeStatus::Pending {
                uuid: details.uuid.ok_or_else(|| {
                    anyhow::anyhow!("GitHub returned a pending async merge without a UUID")
                })?,
            }),
            "merged" => Ok(AsyncMergeStatus::Merged { sha: details.sha }),
            "enqueued" => Ok(AsyncMergeStatus::Enqueued {
                message: details.message,
            }),
            "failed" => Ok(AsyncMergeStatus::Failed {
                message: details.message,
            }),
            status => Err(anyhow::anyhow!(
                "GitHub returned unknown async merge status: {status}"
            )),
        }
    }
}

/// Stack membership metadata exposed on a pull request resource.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PullRequestStack {
    pub number: u64,
    pub position: u64,
    pub size: u64,
}

/// Operations for GitHub pull request stacks.
pub trait PrStackClient: Send + Sync {
    /// Get the stack containing a pull request, if it belongs to one.
    async fn get_stack_for_pull_request(
        &self,
        owner: &str,
        repo: &str,
        pull_request: u64,
    ) -> Result<Option<PullRequestStack>>;

    /// Create a stack from bottom to top.
    async fn create_stack(&self, owner: &str, repo: &str, pull_requests: &[u64]) -> Result<()>;

    /// Add a pull request to the top of a stack.
    async fn add_pull_request_to_stack(
        &self,
        owner: &str,
        repo: &str,
        stack_number: u64,
        pull_request: u64,
    ) -> Result<()>;
}

#[derive(Debug, Deserialize)]
struct PullRequestResponse {
    stack: Option<PullRequestStack>,
}

impl GitHubClient {
    /// Request a squash merge and wait until GitHub reports a final result.
    pub async fn merge_pull_request_async(
        &self,
        owner: &str,
        repo: &str,
        pull_request: u64,
        head_sha: &str,
    ) -> Result<AsyncMergeResult> {
        let route = format!("/repos/{owner}/{repo}/pulls/{pull_request}/merge-async");
        let response = self
            .http
            .put(self.url(&route))
            .json(&json!({ "sha": head_sha, "merge_method": "squash" }))
            .send()
            .await?;
        let status_code = response.status();
        if !status_code.is_success() && status_code != reqwest::StatusCode::CONFLICT {
            let body: serde_json::Value = response
                .json()
                .await
                .unwrap_or_else(|_| json!({ "message": "Unknown error" }));
            return Err(api_error_from_response(status_code.as_u16(), &body).into());
        }

        let mut status = response.json::<AsyncMergeResponse>().await?.into_status()?;
        loop {
            match status {
                AsyncMergeStatus::Pending { uuid } => {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
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
                    return Err(anyhow::anyhow!("Asynchronous merge failed: {message}"));
                }
            }
        }
    }
}

impl PrStackClient for GitHubClient {
    async fn get_stack_for_pull_request(
        &self,
        owner: &str,
        repo: &str,
        pull_request: u64,
    ) -> Result<Option<PullRequestStack>> {
        let route = format!("/repos/{owner}/{repo}/pulls/{pull_request}");
        let pull_request: PullRequestResponse = self.rest_get(&route).await?;

        Ok(pull_request.stack)
    }

    async fn create_stack(&self, owner: &str, repo: &str, pull_requests: &[u64]) -> Result<()> {
        let route = format!("/repos/{owner}/{repo}/stacks");
        let payload = json!({ "pull_requests": pull_requests });
        let _: serde_json::Value = self.rest_post(&route, &payload).await?;
        Ok(())
    }

    async fn add_pull_request_to_stack(
        &self,
        owner: &str,
        repo: &str,
        stack_number: u64,
        pull_request: u64,
    ) -> Result<()> {
        let route = format!("/repos/{owner}/{repo}/stacks/{stack_number}/add");
        let payload = json!({ "pull_requests": [pull_request] });
        let _: serde_json::Value = self.rest_post(&route, &payload).await?;
        Ok(())
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
            .merge_pull_request_async("example-org", "example-repo", 42, "head-sha")
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
            .merge_pull_request_async("example-org", "example-repo", 42, "head-sha")
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
            .merge_pull_request_async("example-org", "example-repo", 42, "head-sha")
            .await;

        assert_eq!(
            result.map_err(|error| error.to_string()),
            Err("Asynchronous merge failed: Required checks are pending.".into()),
        );
    }
}
