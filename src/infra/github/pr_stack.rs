//! Pull request stack operations.

use serde::Deserialize;
use serde_json::json;

use super::client::GitHubClient;
use super::error::Result;

/// A pull request stack and its bottom-to-top pull request order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestStack {
    pub number: u64,
    pub pull_requests: Vec<u64>,
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
struct StackResponse {
    number: u64,
    pull_requests: Vec<StackPullRequestResponse>,
}

#[derive(Debug, Deserialize)]
struct StackPullRequestResponse {
    number: u64,
}

impl PrStackClient for GitHubClient {
    async fn get_stack_for_pull_request(
        &self,
        owner: &str,
        repo: &str,
        pull_request: u64,
    ) -> Result<Option<PullRequestStack>> {
        let route = format!("/repos/{owner}/{repo}/stacks");
        let pull_request = pull_request.to_string();
        let stacks: Vec<StackResponse> = self
            .rest_get_with_query(&route, &[("pull_request", &pull_request)])
            .await?;

        Ok(stacks.into_iter().next().map(|stack| PullRequestStack {
            number: stack.number,
            pull_requests: stack
                .pull_requests
                .into_iter()
                .map(|pull_request| pull_request.number)
                .collect(),
        }))
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
