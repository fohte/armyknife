//! Pull request stack operations.

use serde::Deserialize;
use serde_json::json;

use super::client::GitHubClient;
use super::error::Result;

/// Stack membership metadata exposed on a pull request resource.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PullRequestStack {
    pub number: u64,
    pub position: u64,
    pub size: u64,
}

/// Pull request state returned in stack order by the Stacks API.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PullRequestStackEntry {
    pub number: u64,
    pub merged_at: Option<String>,
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

#[derive(Debug, Deserialize)]
struct PullRequestStackResponse {
    pull_requests: Vec<PullRequestStackEntry>,
}

impl GitHubClient {
    /// Get the pull requests in a stack, ordered from bottom to top.
    pub async fn get_pull_requests_in_stack(
        &self,
        owner: &str,
        repo: &str,
        stack_number: u64,
    ) -> Result<Vec<PullRequestStackEntry>> {
        let route = format!("/repos/{owner}/{repo}/stacks/{stack_number}");
        let stack: PullRequestStackResponse = self.rest_get(&route).await?;
        Ok(stack.pull_requests)
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
        matchers::{method, path},
    };

    use super::*;

    #[rstest::rstest]
    #[tokio::test]
    async fn get_pull_requests_in_stack_returns_members_in_api_order() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/example-org/example-repo/stacks/88"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "pull_requests": [
                    { "number": 41, "state": "closed", "merged_at": "2026-01-02T03:04:05Z" },
                    { "number": 42, "state": "open", "merged_at": null },
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = GitHubClient::with_base_url(&server.uri(), "test-token").unwrap();

        let result = client
            .get_pull_requests_in_stack("example-org", "example-repo", 88)
            .await;

        assert_eq!(
            result.map_err(|error| error.to_string()),
            Ok(vec![
                PullRequestStackEntry {
                    number: 41,
                    merged_at: Some("2026-01-02T03:04:05Z".to_string()),
                },
                PullRequestStackEntry {
                    number: 42,
                    merged_at: None,
                },
            ]),
        );
    }
}
