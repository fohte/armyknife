//! Wait for an existing review from a bot reviewer.

use super::client::{ReviewClient, get_client};
use super::common::{get_pr_number, get_repo_owner_and_name};
use super::error::{Result, ReviewError};
use super::reviewer::Reviewer;
use chrono::{DateTime, Utc};
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
    run_with_client(args, &get_client()).await
}

pub async fn run_with_client(args: &WaitArgs, client: &dyn ReviewClient) -> Result<()> {
    let (owner, repo) = get_repo_owner_and_name(args.repo.as_deref())?;
    let pr_number = get_pr_number(&owner, &repo, args.pr).await?;

    run_wait(args, client, &owner, &repo, pr_number).await
}

/// Internal implementation that can be tested without PR/repo detection.
pub(crate) async fn run_wait(
    args: &WaitArgs,
    client: &dyn ReviewClient,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> Result<()> {
    println!("Checking PR #{pr_number} for {:?} review...", args.reviewer);

    // Record start time to detect "new" reviews
    let start_time = Utc::now();

    // Check if reviewer has already posted a review
    let existing_review = client
        .find_latest_review(owner, repo, pr_number, args.reviewer)
        .await?;

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
    let past_time = DateTime::<Utc>::MIN_UTC;
    if let Some(unable_msg) = client
        .check_reviewer_unable_comment(owner, repo, pr_number, past_time, args.reviewer)
        .await?
    {
        return Err(ReviewError::ReviewerUnable(unable_msg));
    }

    // Check if review has started by looking for any comment from the reviewer
    if !client
        .has_reviewer_activity(owner, repo, pr_number, args.reviewer)
        .await?
    {
        return Err(ReviewError::ReviewNotStarted);
    }

    println!("Waiting for {:?} review to complete...", args.reviewer);

    // Poll for new review
    wait_for_review(client, owner, repo, pr_number, start_time, args).await?;

    println!("\n{:?} review completed!", args.reviewer);
    Ok(())
}

/// Poll until reviewer posts a review after start_time
async fn wait_for_review(
    client: &dyn ReviewClient,
    owner: &str,
    repo: &str,
    pr_number: u64,
    start_time: DateTime<Utc>,
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
        if let Some(review_time) = client
            .find_latest_review(owner, repo, pr_number, args.reviewer)
            .await?
            && review_time > start_time
        {
            return Ok(());
        }

        // Check if reviewer posted an "unable to" comment
        if let Some(unable_msg) = client
            .check_reviewer_unable_comment(owner, repo, pr_number, start_time, args.reviewer)
            .await?
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::review::client::mock::MockReviewClient;
    use chrono::Duration as ChronoDuration;
    use rstest::rstest;

    fn make_args(interval: u64, timeout: u64) -> WaitArgs {
        WaitArgs {
            pr: Some(1),
            repo: Some("owner/repo".to_string()),
            reviewer: Reviewer::Gemini,
            interval,
            timeout,
        }
    }

    #[tokio::test]
    async fn wait_already_completed() {
        // Scenario: Review already exists
        let review_time = Utc::now() - ChronoDuration::hours(1);

        let client = MockReviewClient::new().with_review(Reviewer::Gemini, review_time);

        let args = make_args(1, 5);
        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wait_review_not_started() {
        // Scenario: No review and no activity from reviewer
        let client = MockReviewClient::new();

        let args = make_args(1, 5);
        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        assert!(matches!(result, Err(ReviewError::ReviewNotStarted)));
    }

    #[tokio::test]
    async fn wait_in_progress_then_completes() {
        // Scenario: Reviewer has activity (started) but no review yet, then completes
        let review_time = Utc::now() + ChronoDuration::seconds(1);

        let client = MockReviewClient::new()
            .with_comment("gemini-code-assist", "Starting review...", Utc::now())
            .with_review(Reviewer::Gemini, review_time);

        let args = make_args(1, 5);
        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wait_unable_to_review_existing() {
        // Scenario: "Unable to" comment already exists
        let unable_time = Utc::now() - ChronoDuration::hours(1);

        let client = MockReviewClient::new().with_comment(
            "gemini-code-assist",
            "Gemini is unable to review this PR.",
            unable_time,
        );

        let args = make_args(1, 5);
        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        assert!(matches!(result, Err(ReviewError::ReviewerUnable(_))));
    }

    #[tokio::test]
    async fn wait_unable_to_review_during_wait() {
        // Scenario: Review started, then "unable to" comment appears during wait
        let unable_time = Utc::now() + ChronoDuration::milliseconds(100);

        let client = MockReviewClient::new()
            .with_comment("gemini-code-assist", "Starting review...", Utc::now())
            .with_comment(
                "gemini-code-assist",
                "Gemini is unable to review this PR.",
                unable_time,
            );

        let args = make_args(1, 5);
        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        assert!(matches!(result, Err(ReviewError::ReviewerUnable(_))));
    }

    #[tokio::test]
    async fn wait_timeout() {
        // Scenario: Review started but never completes
        let client = MockReviewClient::new().with_comment(
            "gemini-code-assist",
            "Starting review...",
            Utc::now(),
        );

        let args = make_args(1, 1); // 1 second timeout
        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        assert!(matches!(result, Err(ReviewError::Timeout(1))));
    }

    #[rstest]
    #[case::gemini(Reviewer::Gemini)]
    #[tokio::test]
    async fn wait_detects_correct_reviewer(#[case] reviewer: Reviewer) {
        // Scenario: Only detect reviews from the specified reviewer
        let review_time = Utc::now() - ChronoDuration::hours(1);

        let client = MockReviewClient::new().with_review(reviewer, review_time);

        let args = WaitArgs {
            pr: Some(1),
            repo: Some("owner/repo".to_string()),
            reviewer,
            interval: 1,
            timeout: 5,
        };

        let result = run_wait(&args, &client, "owner", "repo", 1).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wait_ignores_other_reviewers_activity() {
        // Scenario: Another bot has activity, but not the requested reviewer
        let client = MockReviewClient::new().with_comment("other-bot", "Some comment", Utc::now());

        let args = make_args(1, 5);
        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        // Should fail because gemini-code-assist has no activity
        assert!(matches!(result, Err(ReviewError::ReviewNotStarted)));
    }
}
