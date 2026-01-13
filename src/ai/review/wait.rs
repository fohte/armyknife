//! Wait for an existing review from a bot reviewer.

use super::common::{
    check_reviewer_unable_comment, find_latest_review, get_pr_number, get_repo_owner_and_name,
};
use super::error::{Result, ReviewError};
use super::reviewer::Reviewer;
use chrono::Utc;
use clap::Args;
use std::io::Write;
use std::time::{Duration, Instant};

const POLL_INTERVAL_SECS: u64 = 15;
const TIMEOUT_SECS: u64 = 300; // 5 minutes

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WaitArgs {
    /// PR number (auto-detects from current branch if not specified)
    pub pr: Option<u64>,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,

    /// Reviewer to wait for
    #[arg(short = 'r', long = "reviewer", value_enum, default_value = "gemini")]
    pub reviewer: Reviewer,

    /// Polling interval in seconds
    #[arg(long, default_value_t = POLL_INTERVAL_SECS)]
    pub interval: u64,

    /// Timeout in seconds
    #[arg(long, default_value_t = TIMEOUT_SECS)]
    pub timeout: u64,
}

pub async fn run(args: &WaitArgs) -> Result<()> {
    let (owner, repo) = get_repo_owner_and_name(args.repo.as_deref())?;
    let pr_number = get_pr_number(&owner, &repo, args.pr).await?;

    println!("Checking PR #{pr_number} for {:?} review...", args.reviewer);

    // Record start time to detect "new" reviews
    let start_time = Utc::now();

    // Check if reviewer has already posted a review
    let existing_review = find_latest_review(&owner, &repo, pr_number, args.reviewer).await?;

    if let Some(review_time) = existing_review {
        // Review already completed
        println!(
            "{:?} review already completed at {}",
            args.reviewer,
            review_time.format("%Y-%m-%d %H:%M:%S UTC")
        );
        return Ok(());
    }

    // Check if reviewer already posted an "unable to" comment
    // Use a time in the past to check for any existing "unable" comment
    let past_time = chrono::DateTime::<Utc>::MIN_UTC;
    if let Some(unable_msg) =
        check_reviewer_unable_comment(&owner, &repo, pr_number, past_time, args.reviewer)?
    {
        return Err(ReviewError::ReviewerUnable(unable_msg));
    }

    // Check if review has started by looking for any comment from the reviewer
    if !has_reviewer_activity(&owner, &repo, pr_number, args.reviewer)? {
        return Err(ReviewError::ReviewNotStarted);
    }

    println!("Waiting for {:?} review to complete...", args.reviewer);

    // Poll for new review
    wait_for_review(&owner, &repo, pr_number, start_time, args).await?;

    println!("\n{:?} review completed!", args.reviewer);
    Ok(())
}

/// Check if the reviewer has any activity (comments) on the PR
fn has_reviewer_activity(
    owner: &str,
    repo: &str,
    pr_number: u64,
    reviewer: Reviewer,
) -> Result<bool> {
    let bot_login = reviewer.bot_login();

    let output = std::process::Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "comments",
            "--jq",
            &format!(r#".comments[] | select(.author.login == "{bot_login}") | .author.login"#),
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
    Ok(!stdout.trim().is_empty())
}

/// Poll until reviewer posts a review after start_time
async fn wait_for_review(
    owner: &str,
    repo: &str,
    pr_number: u64,
    start_time: chrono::DateTime<Utc>,
    args: &WaitArgs,
) -> Result<()> {
    let poll_interval = Duration::from_secs(args.interval);
    let timeout = Duration::from_secs(args.timeout);
    let started_at = Instant::now();

    loop {
        // Check timeout
        let elapsed = started_at.elapsed();
        if elapsed >= timeout {
            return Err(ReviewError::Timeout(args.timeout));
        }

        // Check for new review
        if let Some(review_time) = find_latest_review(owner, repo, pr_number, args.reviewer).await?
            && review_time > start_time
        {
            return Ok(());
        }

        // Check if reviewer posted an "unable to" comment
        if let Some(unable_msg) =
            check_reviewer_unable_comment(owner, repo, pr_number, start_time, args.reviewer)?
        {
            return Err(ReviewError::ReviewerUnable(unable_msg));
        }

        // Print progress
        let elapsed_secs = elapsed.as_secs();
        print!(
            "\rWaiting for {:?} review... ({elapsed_secs}s elapsed, timeout: {}s)   ",
            args.reviewer, args.timeout
        );
        std::io::stdout().flush().ok();

        // Wait before next poll
        tokio::time::sleep(poll_interval).await;
    }
}
