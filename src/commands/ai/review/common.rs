//! Common utilities for review commands.

use super::client::ReviewClient;
use super::error::{Result, ReviewError};
use super::reviewer::Reviewer;
use crate::infra::git;
use crate::infra::github::{OctocrabClient, PrClient, PrState};
use chrono::{DateTime, Utc};
use std::io::Write;
use std::time::{Duration, Instant};

/// Get repository owner and name from argument or git remote
pub fn get_repo_owner_and_name(repo_arg: Option<&str>) -> Result<(String, String)> {
    if let Some(repo) = repo_arg {
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() == 2 {
            return Ok((parts[0].to_string(), parts[1].to_string()));
        }
        return Err(ReviewError::RepoInfoError(format!(
            "Invalid repository format: {repo}. Expected owner/repo"
        ))
        .into());
    }

    let repo = git::open_repo()?;
    let (owner, name) = git::github_owner_and_repo(&repo)?;
    Ok((owner, name))
}

/// Get PR number from argument or by finding PR for current branch
pub async fn get_pr_number(owner: &str, repo: &str, pr_arg: Option<u64>) -> Result<u64> {
    if let Some(pr) = pr_arg {
        return Ok(pr);
    }

    // Get current branch name
    let git_repo = git::open_repo()?;
    let branch = git::current_branch(&git_repo)?;

    // Find PR for this branch
    let client = OctocrabClient::get()?;
    let pr_info = client.get_pr_for_branch(owner, repo, &branch).await?;

    match pr_info {
        Some(info) if info.state == PrState::Open => {
            // Extract PR number from URL (e.g., https://github.com/owner/repo/pull/123)
            let pr_number = info
                .url
                .rsplit('/')
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| {
                    ReviewError::RepoInfoError("Failed to parse PR number from URL".into())
                })?;
            Ok(pr_number)
        }
        _ => Err(ReviewError::NoPrFound.into()),
    }
}

/// Configuration for polling a review
pub struct WaitConfig {
    pub reviewer: Reviewer,
    pub interval: u64,
    pub timeout: u64,
}

/// Poll until reviewer posts a review after start_time
pub async fn wait_for_review(
    client: &dyn ReviewClient,
    owner: &str,
    repo: &str,
    pr_number: u64,
    start_time: DateTime<Utc>,
    config: &WaitConfig,
) -> Result<()> {
    let poll_interval = Duration::from_secs(config.interval);
    let timeout_duration = Duration::from_secs(config.timeout);
    let started_at = Instant::now();

    loop {
        // Check timeout
        let elapsed = started_at.elapsed();
        if elapsed >= timeout_duration {
            return Err(ReviewError::Timeout(config.timeout).into());
        }

        // Check for new review
        if let Some(review_time) = client
            .find_latest_review(owner, repo, pr_number, config.reviewer)
            .await?
            && review_time > start_time
        {
            return Ok(());
        }

        // Check if reviewer posted an "unable to" comment
        if let Some(unable_msg) = client
            .check_reviewer_unable_comment(owner, repo, pr_number, start_time, config.reviewer)
            .await?
        {
            return Err(ReviewError::ReviewerUnable(unable_msg).into());
        }

        // Print progress
        let elapsed_secs = elapsed.as_secs();
        print!(
            "\rWaiting for {:?} review... ({elapsed_secs}s elapsed, timeout: {}s)   ",
            config.reviewer, config.timeout
        );
        std::io::stdout().flush().ok();

        // Wait before next poll
        tokio::time::sleep(poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::valid("owner/repo", "owner", "repo")]
    #[case::with_dashes("my-org/my-repo", "my-org", "my-repo")]
    #[case::with_numbers("user123/project456", "user123", "project456")]
    fn test_get_repo_owner_and_name_with_arg(
        #[case] input: &str,
        #[case] expected_owner: &str,
        #[case] expected_repo: &str,
    ) {
        let (owner, repo) = get_repo_owner_and_name(Some(input)).unwrap();
        assert_eq!(owner, expected_owner);
        assert_eq!(repo, expected_repo);
    }

    #[rstest]
    #[case::no_slash("invalid")]
    #[case::too_many_slashes("a/b/c")]
    #[case::empty("")]
    fn test_get_repo_owner_and_name_invalid(#[case] input: &str) {
        let result = get_repo_owner_and_name(Some(input));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.downcast_ref::<ReviewError>()
                .is_some_and(|e| matches!(e, ReviewError::RepoInfoError(_)))
        );
    }
}
