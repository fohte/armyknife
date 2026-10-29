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
