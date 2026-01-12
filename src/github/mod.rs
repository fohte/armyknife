//! GitHub API client module using octocrab.
//!
//! Provides a thin wrapper around octocrab for GitHub operations,
//! with authentication via `gh auth token`.

use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitHubError {
    #[error("Failed to get GitHub token: {0}")]
    TokenError(String),

    #[error("GitHub API error: {0}")]
    ApiError(#[from] octocrab::Error),
}

pub type Result<T> = std::result::Result<T, GitHubError>;

/// Get GitHub token from `gh auth token` command.
/// This reuses the authentication from GitHub CLI.
pub fn get_gh_token() -> Result<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map_err(|e| GitHubError::TokenError(format!("Failed to run gh auth token: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitHubError::TokenError(format!(
            "gh auth token failed: {stderr}"
        )));
    }

    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return Err(GitHubError::TokenError(
            "gh auth token returned empty token".to_string(),
        ));
    }

    Ok(token)
}

/// Create an authenticated octocrab instance using gh CLI token.
pub fn create_client() -> Result<octocrab::Octocrab> {
    let token = get_gh_token()?;
    let client = octocrab::Octocrab::builder()
        .personal_token(token)
        .build()
        .map_err(|e| GitHubError::TokenError(format!("Failed to build octocrab client: {e}")))?;
    Ok(client)
}

/// Check if a repository is private.
pub async fn is_repo_private(client: &octocrab::Octocrab, owner: &str, repo: &str) -> Result<bool> {
    let repository = client.repos(owner, repo).get().await?;
    Ok(repository.private.unwrap_or(false))
}

/// Parameters for creating a pull request.
#[derive(Debug, Clone)]
pub struct CreatePrParams {
    pub owner: String,
    pub repo: String,
    pub title: String,
    pub body: String,
    pub head: String,
    pub base: Option<String>,
    pub draft: bool,
}

/// Create a pull request and return its URL.
pub async fn create_pull_request(
    client: &octocrab::Octocrab,
    params: CreatePrParams,
) -> Result<String> {
    let pulls = client.pulls(&params.owner, &params.repo);
    let base = params.base.as_deref().unwrap_or("main");

    let pr = if params.draft {
        pulls
            .create(&params.title, &params.head, base)
            .body(&params.body)
            .draft(Some(true))
            .send()
            .await?
    } else {
        pulls
            .create(&params.title, &params.head, base)
            .body(&params.body)
            .send()
            .await?
    };

    Ok(pr.html_url.map(|u| u.to_string()).unwrap_or_default())
}

/// Open a URL in the default browser.
pub fn open_in_browser(url: &str) {
    let _ = open::that(url);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_gh_token_format() {
        // This test only runs if gh is authenticated
        if let Ok(token) = get_gh_token() {
            assert!(!token.is_empty());
            assert!(!token.contains('\n'));
        }
    }
}
