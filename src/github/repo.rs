//! Repository operations.

use super::client::OctocrabClient;
use super::error::Result;

/// Trait for repository operations.
#[async_trait::async_trait]
pub trait RepoClient: Send + Sync {
    /// Check if a repository is private.
    async fn is_repo_private(&self, owner: &str, repo: &str) -> Result<bool>;

    /// Get the default branch name from GitHub API.
    async fn get_default_branch(&self, owner: &str, repo: &str) -> Result<String>;
}

#[async_trait::async_trait]
impl RepoClient for OctocrabClient {
    async fn is_repo_private(&self, owner: &str, repo: &str) -> Result<bool> {
        let repository = self.client.repos(owner, repo).get().await?;
        // Default to private for safety (e.g., to avoid incorrectly flagging
        // Japanese text in a private repo as an error)
        Ok(repository.private.unwrap_or(true))
    }

    async fn get_default_branch(&self, owner: &str, repo: &str) -> Result<String> {
        let repository = self.client.repos(owner, repo).get().await?;
        Ok(repository
            .default_branch
            .unwrap_or_else(|| "main".to_string()))
    }
}
