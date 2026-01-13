//! Wait for Gemini Code Assist review on a PR.

mod error;

pub use error::{Result, ReviewGeminiError};

use crate::gh::check_pr_review::fetch_pr_data;
use crate::git;
use crate::github::{OctocrabClient, PrClient, PrState};
use chrono::{DateTime, Utc};
use clap::Args;
use std::io::Write;
use std::process::Command;
use std::time::{Duration, Instant};

const GEMINI_BOT_LOGIN: &str = "gemini-code-assist";
const GEMINI_REVIEW_COMMAND: &str = "/gemini review";
const GEMINI_UNABLE_MARKER: &str = "Gemini is unable to";
const POLL_INTERVAL_SECS: u64 = 15;
const TIMEOUT_SECS: u64 = 300; // 5 minutes

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReviewGeminiArgs {
    /// PR number (auto-detects from current branch if not specified)
    pub pr: Option<u64>,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,

    /// Polling interval in seconds
    #[arg(long, default_value_t = POLL_INTERVAL_SECS)]
    pub interval: u64,

    /// Timeout in seconds
    #[arg(long, default_value_t = TIMEOUT_SECS)]
    pub timeout: u64,
}

pub async fn run(args: &ReviewGeminiArgs) -> Result<()> {
    let (owner, repo) = get_repo_owner_and_name(args.repo.as_deref())?;
    let pr_number = get_pr_number(&owner, &repo, args.pr).await?;

    println!("Checking PR #{pr_number} for Gemini review...");

    // Record start time to detect "new" reviews
    let start_time = Utc::now();

    // Check if Gemini has already posted a review
    let existing_review = find_latest_gemini_review(&owner, &repo, pr_number).await?;

    if existing_review.is_none() {
        // First call: Gemini hasn't reviewed yet, just wait
        println!("Waiting for Gemini Code Assist to post initial review...");
    } else {
        // Subsequent call: Post /gemini review comment to trigger re-review
        println!("Posting /gemini review command...");
        post_gemini_review_comment(&owner, &repo, pr_number)?;
    }

    // Poll for new Gemini review
    wait_for_gemini_review(&owner, &repo, pr_number, start_time, args).await?;

    println!("\nGemini review completed!");
    Ok(())
}

/// Get repository owner and name from argument or git remote
fn get_repo_owner_and_name(repo_arg: Option<&str>) -> Result<(String, String)> {
    if let Some(repo) = repo_arg {
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() == 2 {
            return Ok((parts[0].to_string(), parts[1].to_string()));
        }
        return Err(ReviewGeminiError::RepoInfoError(format!(
            "Invalid repository format: {repo}. Expected owner/repo"
        )));
    }

    let repo = git::open_repo()?;
    let (owner, name) = git::github_owner_and_repo(&repo)?;
    Ok((owner, name))
}

/// Get PR number from argument or by finding PR for current branch
async fn get_pr_number(owner: &str, repo: &str, pr_arg: Option<u64>) -> Result<u64> {
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
                    ReviewGeminiError::RepoInfoError("Failed to parse PR number from URL".into())
                })?;
            Ok(pr_number)
        }
        _ => Err(ReviewGeminiError::NoPrFound),
    }
}

/// Find the latest Gemini review timestamp
async fn find_latest_gemini_review(
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> Result<Option<DateTime<Utc>>> {
    let pr_data = fetch_pr_data(owner, repo, pr_number, true)
        .await
        .map_err(|e| ReviewGeminiError::RepoInfoError(e.to_string()))?;

    let mut latest: Option<DateTime<Utc>> = None;

    for review in &pr_data.reviews {
        if let Some(author) = &review.author
            && author.login == GEMINI_BOT_LOGIN
        {
            let created_at: DateTime<Utc> = review
                .created_at
                .parse()
                .map_err(|_| ReviewGeminiError::TimestampParseError(review.created_at.clone()))?;

            latest = Some(match latest {
                Some(prev) if created_at > prev => created_at,
                Some(prev) => prev,
                None => created_at,
            });
        }
    }

    Ok(latest)
}

/// Check if Gemini posted an "unable to" comment after start_time
fn check_gemini_unable_comment(
    owner: &str,
    repo: &str,
    pr_number: u64,
    start_time: DateTime<Utc>,
) -> Result<Option<String>> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "comments",
            "--jq",
            &format!(
                r#".comments[] | select(.author.login == "{GEMINI_BOT_LOGIN}") | {{body: .body, createdAt: .createdAt}}"#
            ),
            "-R",
            &format!("{owner}/{repo}"),
        ])
        .output()
        .map_err(|e| ReviewGeminiError::CommentError(format!("Failed to run gh: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ReviewGeminiError::CommentError(format!(
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
                && body.contains(GEMINI_UNABLE_MARKER)
            {
                return Ok(Some(body.to_string()));
            }
        }
    }

    Ok(None)
}

/// Post /gemini review comment using gh CLI
fn post_gemini_review_comment(owner: &str, repo: &str, pr_number: u64) -> Result<()> {
    let output = Command::new("gh")
        .args([
            "pr",
            "comment",
            &pr_number.to_string(),
            "--body",
            GEMINI_REVIEW_COMMAND,
            "-R",
            &format!("{owner}/{repo}"),
        ])
        .output()
        .map_err(|e| ReviewGeminiError::CommentError(format!("Failed to run gh: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ReviewGeminiError::CommentError(format!(
            "gh pr comment failed: {stderr}"
        )));
    }

    Ok(())
}

/// Poll until Gemini posts a review after start_time
async fn wait_for_gemini_review(
    owner: &str,
    repo: &str,
    pr_number: u64,
    start_time: DateTime<Utc>,
    args: &ReviewGeminiArgs,
) -> Result<()> {
    let poll_interval = Duration::from_secs(args.interval);
    let timeout = Duration::from_secs(args.timeout);
    let started_at = Instant::now();

    loop {
        // Check timeout
        let elapsed = started_at.elapsed();
        if elapsed >= timeout {
            return Err(ReviewGeminiError::Timeout(args.timeout));
        }

        // Check for new Gemini review
        if let Some(review_time) = find_latest_gemini_review(owner, repo, pr_number).await?
            && review_time > start_time
        {
            return Ok(());
        }

        // Check if Gemini posted an "unable to" comment
        if let Some(unable_msg) = check_gemini_unable_comment(owner, repo, pr_number, start_time)? {
            return Err(ReviewGeminiError::GeminiUnable(unable_msg));
        }

        // Print progress
        let elapsed_secs = elapsed.as_secs();
        print!(
            "\rWaiting for Gemini review... ({elapsed_secs}s elapsed, timeout: {}s)   ",
            args.timeout
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
        assert!(matches!(result, Err(ReviewGeminiError::RepoInfoError(_))));
    }
}
