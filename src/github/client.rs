//! GitHub API client implementation using octocrab.

use std::process::Command;
use std::sync::OnceLock;

use super::error::{GitHubError, Result};

/// Production implementation using octocrab.
pub struct OctocrabClient {
    pub(crate) client: octocrab::Octocrab,
}

/// Global singleton instance of OctocrabClient, initialized lazily.
///
/// Stores the `Result` of initialization. Using a single `OnceLock` for the result
/// ensures initialization logic runs only once, even across multiple threads.
static OCTOCRAB_CLIENT: OnceLock<std::result::Result<OctocrabClient, String>> = OnceLock::new();

impl OctocrabClient {
    /// Create a new OctocrabClient instance.
    /// Prefer using `OctocrabClient::get()` to reuse the singleton instance.
    fn new() -> Result<Self> {
        let token = get_gh_token()?;
        let client = octocrab::Octocrab::builder()
            .personal_token(token)
            .build()
            .map_err(|e| {
                GitHubError::TokenError(format!("Failed to build octocrab client: {e}"))
            })?;
        Ok(Self { client })
    }

    /// Get the singleton instance of OctocrabClient.
    /// Initializes the client on first call (runs `gh auth token` once).
    pub fn get() -> Result<&'static Self> {
        // get_or_init ensures the closure is only run once across all threads
        let result = OCTOCRAB_CLIENT.get_or_init(|| Self::new().map_err(|e| e.to_string()));

        match result {
            Ok(client) => Ok(client),
            Err(e) => Err(GitHubError::TokenError(e.clone())),
        }
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
