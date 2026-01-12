//! GitHub API client implementation using octocrab.

use std::process::Command;

use super::error::{GitHubError, Result};

/// Production implementation using octocrab.
pub struct OctocrabClient {
    pub(crate) client: octocrab::Octocrab,
}

impl OctocrabClient {
    pub fn new() -> Result<Self> {
        let token = get_gh_token()?;
        let client = octocrab::Octocrab::builder()
            .personal_token(token)
            .build()
            .map_err(|e| {
                GitHubError::TokenError(format!("Failed to build octocrab client: {e}"))
            })?;
        Ok(Self { client })
    }
}

/// Get GitHub token from `gh auth token` command.
/// This reuses the authentication from GitHub CLI.
fn get_gh_token() -> Result<String> {
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
