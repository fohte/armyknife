//! Common utilities for review commands.

use super::error::{Result, ReviewError};
use super::reviewer::Reviewer;
use crate::gh::check_pr_review::fetch_pr_data;
use crate::git;
use crate::github::{OctocrabClient, PrClient, PrState};
use chrono::{DateTime, Utc};
use std::process::Command;

/// Get repository owner and name from argument or git remote
pub fn get_repo_owner_and_name(repo_arg: Option<&str>) -> Result<(String, String)> {
    if let Some(repo) = repo_arg {
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() == 2 {
            return Ok((parts[0].to_string(), parts[1].to_string()));
        }
        return Err(ReviewError::RepoInfoError(format!(
            "Invalid repository format: {repo}. Expected owner/repo"
        )));
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
        _ => Err(ReviewError::NoPrFound),
    }
}

/// Find the latest review timestamp from the specified reviewer
pub async fn find_latest_review(
    owner: &str,
    repo: &str,
    pr_number: u64,
    reviewer: Reviewer,
) -> Result<Option<DateTime<Utc>>> {
    let pr_data = fetch_pr_data(owner, repo, pr_number, true)
        .await
        .map_err(|e| ReviewError::RepoInfoError(e.to_string()))?;

    let bot_login = reviewer.bot_login();
    let mut latest: Option<DateTime<Utc>> = None;

    for review in &pr_data.reviews {
        if let Some(author) = &review.author
            && author.login == bot_login
        {
            let created_at: DateTime<Utc> = review
                .created_at
                .parse()
                .map_err(|_| ReviewError::TimestampParseError(review.created_at.clone()))?;

            latest = Some(match latest {
                Some(prev) if created_at > prev => created_at,
                Some(prev) => prev,
                None => created_at,
            });
        }
    }

    Ok(latest)
}

/// Check if the reviewer posted an "unable to" comment after start_time
pub fn check_reviewer_unable_comment(
    owner: &str,
    repo: &str,
    pr_number: u64,
    start_time: DateTime<Utc>,
    reviewer: Reviewer,
) -> Result<Option<String>> {
    let bot_login = reviewer.bot_login();
    let unable_marker = reviewer.unable_marker();

    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "comments",
            "--jq",
            &format!(
                r#".comments[] | select(.author.login == "{bot_login}") | {{body: .body, createdAt: .createdAt}}"#
            ),
            "-R",
            &format!("{owner}/{repo}"),
        ])
        .output()
        .map_err(|e| ReviewError::CommentError(format!("Failed to run gh: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ReviewError::CommentError(format!(
            "gh pr view failed: {stderr}"
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        if let Ok(comment) = serde_json::from_str::<serde_json::Value>(line) {
            let body = comment["body"].as_str().unwrap_or("");
            let created_at_str = comment["createdAt"].as_str().unwrap_or("");

            if let Ok(created_at) = created_at_str.parse::<DateTime<Utc>>()
                && created_at > start_time
                && body.contains(unable_marker)
            {
                return Ok(Some(body.to_string()));
            }
        }
    }

    Ok(None)
}

/// Post a review request comment using gh CLI
pub fn post_review_comment(
    owner: &str,
    repo: &str,
    pr_number: u64,
    reviewer: Reviewer,
) -> Result<()> {
    let review_command = reviewer.review_command();

    let output = Command::new("gh")
        .args([
            "pr",
            "comment",
            &pr_number.to_string(),
            "--body",
            review_command,
            "-R",
            &format!("{owner}/{repo}"),
        ])
        .output()
        .map_err(|e| ReviewError::CommentError(format!("Failed to run gh: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ReviewError::CommentError(format!(
            "gh pr comment failed: {stderr}"
        )));
    }

    Ok(())
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
        assert!(matches!(result, Err(ReviewError::RepoInfoError(_))));
    }
}
