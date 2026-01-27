//! Request a review from a bot reviewer.

use super::client::{ReviewClient, get_client};
use super::common::{WaitConfig, get_pr_number, get_repo_owner_and_name, wait_for_any_review};
use super::error::Result;
use super::reviewer::Reviewer;
use chrono::Utc;
use clap::Args;

const POLL_INTERVAL_SECS: u64 = 15;
const TIMEOUT_SECS: u64 = 300; // 5 minutes

#[derive(Args, Clone, PartialEq, Eq)]
pub struct RequestArgs {
    /// PR number (auto-detects from current branch if not specified)
    pub pr: Option<u64>,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,

    /// Reviewers to request (can specify multiple, waits for any to complete).
    /// Reviewers that don't support request (like Devin) will be waited for without requesting.
    #[arg(short = 'r', long = "reviewer", value_enum, default_values_t = vec![Reviewer::Gemini, Reviewer::Devin])]
    pub reviewers: Vec<Reviewer>,

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
    let reviewers = &args.reviewers;
    println!("Checking PR #{pr_number} for {:?} review(s)...", reviewers);

    // Record start time to detect "new" reviews
    let start_time = Utc::now();

    // Check if any reviewer has already posted a review
    for reviewer in reviewers {
        if let Some(review_time) = client
            .find_latest_review(owner, repo, pr_number, *reviewer)
            .await?
        {
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
                    reviewer
                );
                return Ok(());
            }
        }
    }

    // Request review from reviewers that support it
    let requestable_reviewers: Vec<_> = reviewers
        .iter()
        .filter(|r| r.review_command().is_some())
        .collect();

    for reviewer in &requestable_reviewers {
        // Check if this reviewer needs a re-review (has old review with new commits)
        let existing_review = client
            .find_latest_review(owner, repo, pr_number, **reviewer)
            .await?;

        if existing_review.is_some() {
            // New commits exist (we already checked above that commit > review),
            // so request re-review
            println!("Posting {:?} review command...", reviewer);
            client
                .post_review_comment(owner, repo, pr_number, **reviewer)
                .await?;
        }
    }

    // Log which reviewers we're waiting for
    let non_requestable: Vec<_> = reviewers
        .iter()
        .filter(|r| r.review_command().is_none())
        .collect();
    if !non_requestable.is_empty() {
        println!(
            "Waiting for {:?} (cannot request, will wait for automatic review)...",
            non_requestable
        );
    }

    println!(
        "Waiting for review to complete from any of {:?}...",
        reviewers
    );

    // Poll for new review from any reviewer
    let config = WaitConfig {
        reviewers: reviewers.clone(),
        interval: args.interval,
        timeout: args.timeout,
    };
    let completed_reviewer =
        wait_for_any_review(client, owner, repo, pr_number, start_time, &config).await?;

    println!("\n{:?} review completed!", completed_reviewer);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ai::review::client::mock::MockReviewClient;
    use chrono::Duration as ChronoDuration;
    use rstest::rstest;

    fn make_args_both(interval: u64, timeout: u64) -> RequestArgs {
        RequestArgs {
            pr: Some(1),
            repo: Some("owner/repo".to_string()),
            reviewers: vec![Reviewer::Gemini, Reviewer::Devin],
            interval,
            timeout,
        }
    }

    /// Helper to build a mock client for success cases.
    fn build_success_client(scenario: &str) -> MockReviewClient {
        let now = Utc::now();
        match scenario {
            "first_review_gemini" => {
                // No existing review, Gemini review appears after polling
                // Skip 4 calls: 2 in first loop (check existing) + 2 in re-review loop
                MockReviewClient::new()
                    .with_review(Reviewer::Gemini, now + ChronoDuration::milliseconds(100))
                    .skip_first_n_review_calls(4)
            }
            "first_review_devin" => {
                // No existing review, Devin review appears after polling
                MockReviewClient::new()
                    .with_review(Reviewer::Devin, now + ChronoDuration::milliseconds(100))
                    .skip_first_n_review_calls(4)
            }
            "already_reviewed_gemini" => {
                // Gemini review exists, commit older than review -> skip
                MockReviewClient::new()
                    .with_review(Reviewer::Gemini, now - ChronoDuration::hours(1))
                    .with_latest_commit_time(now - ChronoDuration::hours(2))
            }
            "already_reviewed_devin" => {
                // Devin review exists, commit older than review -> skip
                MockReviewClient::new()
                    .with_review(Reviewer::Devin, now - ChronoDuration::hours(1))
                    .with_latest_commit_time(now - ChronoDuration::hours(2))
            }
            "re_review_gemini" => {
                // Old Gemini review exists, new commit -> post comment, wait for new review
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
    #[case::first_review_gemini("first_review_gemini", false)]
    #[case::first_review_devin("first_review_devin", false)]
    #[case::already_reviewed_gemini("already_reviewed_gemini", false)]
    #[case::already_reviewed_devin("already_reviewed_devin", false)]
    #[case::re_review_gemini("re_review_gemini", true)]
    #[tokio::test]
    async fn request_succeeds(#[case] scenario: &str, #[case] expects_comment: bool) {
        let client = build_success_client(scenario);
        let args = make_args_both(1, 5);

        let result = run_request(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
        let posted = client.posted_comments.lock().unwrap();
        assert_eq!(!posted.is_empty(), expects_comment);
    }

    #[rstest]
    #[case::timeout(MockReviewClient::new(), 1, "Timeout waiting for review")]
    #[case::unable_to_review(
        MockReviewClient::new().with_comment(
            "gemini-code-assist",
            "Gemini is unable to review this PR.",
            Utc::now() + ChronoDuration::milliseconds(100),
        ),
        5,
        "Reviewer is unable to review this PR"
    )]
    #[tokio::test]
    async fn request_fails(
        #[case] client: MockReviewClient,
        #[case] timeout: u64,
        #[case] expected_error: &str,
    ) {
        let args = make_args_both(1, timeout);
        let result = run_request(&args, &client, "owner", "repo", 1).await;

        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains(expected_error),
            "Expected error containing '{expected_error}', got '{err_msg}'"
        );
    }

    #[tokio::test]
    async fn request_posts_gemini_command_only() {
        // When both reviewers specified, only post command for Gemini (Devin doesn't support it)
        let now = Utc::now();
        let client = MockReviewClient::new()
            .with_review(Reviewer::Gemini, now - ChronoDuration::hours(2))
            .with_review(Reviewer::Gemini, now + ChronoDuration::seconds(1))
            .with_latest_commit_time(now - ChronoDuration::hours(1))
            .with_initial_review_cutoff(now);

        let args = make_args_both(1, 5);

        let result = run_request(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
        let posted = client.posted_comments.lock().unwrap();
        assert_eq!(posted.len(), 1);
        assert_eq!(posted[0].reviewer, Reviewer::Gemini);
    }

    #[tokio::test]
    async fn request_waits_for_devin_without_posting() {
        // When Devin completes first, no comment should be posted for it
        let now = Utc::now();
        let client = MockReviewClient::new()
            .with_review(Reviewer::Devin, now + ChronoDuration::milliseconds(100))
            .skip_first_n_review_calls(4); // Skip initial check loop + re-review check loop

        let args = make_args_both(1, 5);

        let result = run_request(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
        let posted = client.posted_comments.lock().unwrap();
        assert!(posted.is_empty()); // No comments posted for Devin
    }
}
