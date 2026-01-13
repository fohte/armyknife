//! Request a review from a bot reviewer.

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
    run_with_client(args, &get_client()).await
}

pub async fn run_with_client(args: &RequestArgs, client: &dyn ReviewClient) -> Result<()> {
    let (owner, repo) = get_repo_owner_and_name(args.repo.as_deref())?;
    let pr_number = get_pr_number(&owner, &repo, args.pr).await?;

    run_request(args, client, &owner, &repo, pr_number).await
}

/// Internal implementation that can be tested without PR/repo detection.
pub(crate) async fn run_request(
    args: &RequestArgs,
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
        // Check if there are new commits after the last review
        let latest_commit_time = client
            .get_latest_commit_time(owner, repo, pr_number)
            .await?;

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
        client
            .post_review_comment(owner, repo, pr_number, args.reviewer)
            .await?;
    } else {
        // First call: Reviewer hasn't reviewed yet, just wait
        println!("Waiting for {:?} to post initial review...", args.reviewer);
    }

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

    fn make_args(interval: u64, timeout: u64) -> RequestArgs {
        RequestArgs {
            pr: Some(1),
            repo: Some("owner/repo".to_string()),
            reviewer: Reviewer::Gemini,
            interval,
            timeout,
        }
    }

    /// Helper to build a mock client for success cases.
    fn build_success_client(scenario: &str) -> MockReviewClient {
        let now = Utc::now();
        match scenario {
            "first_review" => {
                // No existing review, review appears after polling
                MockReviewClient::new()
                    .with_review(Reviewer::Gemini, now + ChronoDuration::milliseconds(100))
                    .skip_first_n_review_calls(1)
            }
            "already_reviewed" => {
                // Review exists, commit older than review -> skip
                MockReviewClient::new()
                    .with_review(Reviewer::Gemini, now - ChronoDuration::hours(1))
                    .with_latest_commit_time(now - ChronoDuration::hours(2))
            }
            "re_review" => {
                // Old review exists, new commit -> post comment, wait for new review
                MockReviewClient::new()
                    .with_review(Reviewer::Gemini, now - ChronoDuration::hours(2))
                    .with_review(Reviewer::Gemini, now + ChronoDuration::seconds(1))
                    .with_latest_commit_time(now - ChronoDuration::hours(1))
                    .with_initial_review_cutoff(now)
            }
            _ => panic!("Unknown scenario: {scenario}"),
        }
    }

    #[rstest]
    #[case::first_review("first_review", false)]
    #[case::already_reviewed("already_reviewed", false)]
    #[case::re_review("re_review", true)]
    #[tokio::test]
    async fn request_succeeds(#[case] scenario: &str, #[case] expects_comment: bool) {
        let client = build_success_client(scenario);
        let args = make_args(1, 5);

        let result = run_request(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
        let posted = client.posted_comments.lock().unwrap();
        assert_eq!(!posted.is_empty(), expects_comment);
    }

    #[rstest]
    #[case::timeout(MockReviewClient::new(), 1, "Timeout")]
    #[case::unable_to_review(
        MockReviewClient::new().with_comment(
            "gemini-code-assist",
            "Gemini is unable to review this PR.",
            Utc::now() + ChronoDuration::milliseconds(100),
        ),
        5,
        "ReviewerUnable"
    )]
    #[tokio::test]
    async fn request_fails(
        #[case] client: MockReviewClient,
        #[case] timeout: u64,
        #[case] expected_error: &str,
    ) {
        let args = make_args(1, timeout);
        let result = run_request(&args, &client, "owner", "repo", 1).await;

        let err = result.unwrap_err();
        let err_name = format!("{err:?}");
        assert!(
            err_name.starts_with(expected_error),
            "Expected {expected_error}, got {err_name}"
        );
    }

    #[rstest]
    #[case::gemini(Reviewer::Gemini)]
    #[tokio::test]
    async fn request_posts_correct_reviewer_command(#[case] reviewer: Reviewer) {
        let now = Utc::now();
        let client = MockReviewClient::new()
            .with_review(reviewer, now - ChronoDuration::hours(2))
            .with_review(reviewer, now + ChronoDuration::seconds(1))
            .with_latest_commit_time(now - ChronoDuration::hours(1))
            .with_initial_review_cutoff(now);

        let args = RequestArgs {
            pr: Some(42),
            repo: Some("test-owner/test-repo".to_string()),
            reviewer,
            interval: 1,
            timeout: 5,
        };

        let result = run_request(&args, &client, "test-owner", "test-repo", 42).await;

        assert!(result.is_ok());
        let posted = client.posted_comments.lock().unwrap();
        assert_eq!(posted.len(), 1);
        assert_eq!(
            posted[0],
            (
                "test-owner".to_string(),
                "test-repo".to_string(),
                42,
                reviewer
            )
        );
    }
}
