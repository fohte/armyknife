//! Request a review from a bot reviewer.

use super::common::{
    check_reviewer_unable_comment, find_latest_review, get_latest_commit_time, get_pr_number,
    get_repo_owner_and_name, post_review_comment,
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
pub struct RequestArgs {
    /// PR number (auto-detects from current branch if not specified)
    pub pr: Option<u64>,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,

    /// Reviewer to request
    #[arg(short = 'r', long = "reviewer", value_enum, default_value = "gemini")]
    pub reviewer: Reviewer,

    /// Polling interval in seconds
    #[arg(long, default_value_t = POLL_INTERVAL_SECS)]
    pub interval: u64,

    /// Timeout in seconds
    #[arg(long, default_value_t = TIMEOUT_SECS)]
    pub timeout: u64,
}

pub async fn run(args: &RequestArgs) -> Result<()> {
    let (owner, repo) = get_repo_owner_and_name(args.repo.as_deref())?;
    let pr_number = get_pr_number(&owner, &repo, args.pr).await?;

    println!("Checking PR #{pr_number} for {:?} review...", args.reviewer);

    // Record start time to detect "new" reviews
    let start_time = Utc::now();

    // Check if reviewer has already posted a review
    let existing_review = find_latest_review(&owner, &repo, pr_number, args.reviewer).await?;

    if let Some(review_time) = existing_review {
        // Check if there are new commits after the last review
        let latest_commit_time = get_latest_commit_time(&owner, &repo, pr_number).await?;

        if let Some(commit_time) = latest_commit_time
            && commit_time <= review_time
        {
            // No new commits since last review
            println!(
                "{:?} has already reviewed this PR (no new commits since last review)",
                args.reviewer
            );
            return Ok(());
        }

        // New commits exist, request re-review
        println!("Posting {:?} review command...", args.reviewer);
        post_review_comment(&owner, &repo, pr_number, args.reviewer).await?;
    } else {
        // First call: Reviewer hasn't reviewed yet, just wait
        println!("Waiting for {:?} to post initial review...", args.reviewer);
    }

    // Poll for new review
    wait_for_review(&owner, &repo, pr_number, start_time, args).await?;

    println!("\n{:?} review completed!", args.reviewer);
    Ok(())
}

/// Poll until reviewer posts a review after start_time
async fn wait_for_review(
    owner: &str,
    repo: &str,
    pr_number: u64,
    start_time: chrono::DateTime<Utc>,
    args: &RequestArgs,
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
            check_reviewer_unable_comment(owner, repo, pr_number, start_time, args.reviewer).await?
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
