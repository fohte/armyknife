//! Wait for an existing review from a bot reviewer.

use super::client::{ReviewClient, get_client};
use super::common::{WaitConfig, get_pr_number, get_repo_owner_and_name, wait_for_all_reviews};
use super::error::{Result, ReviewError};
use super::reviewer::Reviewer;
use chrono::{DateTime, Utc};
use clap::Args;

const POLL_INTERVAL_SECS: u64 = 15;
const TIMEOUT_SECS: u64 = 300; // 5 minutes

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WaitArgs {
    /// PR number (auto-detects from current branch if not specified)
    pub pr: Option<u64>,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,

    /// Reviewers to wait for (can specify multiple, waits for all to complete)
    #[arg(short = 'r', long = "reviewer", value_enum, default_values_t = vec![Reviewer::Gemini, Reviewer::Devin])]
    pub reviewers: Vec<Reviewer>,

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
    let reviewers = &args.reviewers;
    println!("Checking PR #{pr_number} for {:?} review(s)...", reviewers);

    // Record start time to detect "new" reviews
    let start_time = Utc::now();

    // Check if all reviewers have already posted reviews
    let mut all_completed = true;
    for reviewer in reviewers {
        if let Some(review_time) = client
            .find_latest_review(owner, repo, pr_number, *reviewer)
            .await?
        {
            println!(
                "{:?} review already completed at {}",
                reviewer,
                review_time.format("%Y-%m-%d %H:%M:%S UTC")
            );
        } else {
            all_completed = false;
        }
    }
    if all_completed {
        println!("All reviews already completed!");
        return Ok(());
    }

    // Check if any reviewer already posted an "unable to" comment.
    // Skip unable reviewers instead of erroring, unless all are unable.
    let past_time = DateTime::<Utc>::MIN_UTC;
    let mut skipped_reviewers: Vec<Reviewer> = Vec::new();
    for reviewer in reviewers {
        if let Some(unable_msg) = client
            .check_reviewer_unable_comment(owner, repo, pr_number, past_time, *reviewer)
            .await?
        {
            println!(
                "{:?} is unable to review this PR: {}. Skipping.",
                reviewer, unable_msg
            );
            skipped_reviewers.push(*reviewer);
        }
    }
    if skipped_reviewers.len() == reviewers.len() {
        println!("All reviewers are unable to review this PR. Skipping wait.");
        return Ok(());
    }
    let active_reviewers: Vec<Reviewer> = reviewers
        .iter()
        .filter(|r| !skipped_reviewers.contains(r))
        .copied()
        .collect();

    // Check if review has started for reviewers that require start detection.
    // For reviewers that don't require start detection (e.g., Devin), skip this check.
    let reviewers_requiring_start: Vec<_> = active_reviewers
        .iter()
        .filter(|r| r.requires_start_detection())
        .collect();

    if !reviewers_requiring_start.is_empty() {
        let mut any_started = false;
        for reviewer in &reviewers_requiring_start {
            if client
                .has_reviewer_activity(owner, repo, pr_number, **reviewer)
                .await?
            {
                any_started = true;
                break;
            }
        }

        // If all active reviewers require start detection and none have started, return error.
        // But if some reviewers don't require start detection, we can still wait.
        let has_no_start_detection_reviewers = active_reviewers
            .iter()
            .any(|r| !r.requires_start_detection());

        if !any_started && !has_no_start_detection_reviewers {
            return Err(ReviewError::ReviewNotStarted.into());
        }
    }

    println!(
        "Waiting for all reviews to complete from {:?}...",
        active_reviewers
    );

    // Poll for new reviews from active (non-skipped) reviewers
    let config = WaitConfig {
        reviewers: active_reviewers,
        interval: args.interval,
        timeout: args.timeout,
    };
    wait_for_all_reviews(client, owner, repo, pr_number, start_time, &config).await?;

    println!("\nAll reviews completed!");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ai::review::client::mock::MockReviewClient;
    use chrono::Duration as ChronoDuration;
    use rstest::rstest;

    fn make_args_single(reviewer: Reviewer, interval: u64, timeout: u64) -> WaitArgs {
        WaitArgs {
            pr: Some(1),
            repo: Some("owner/repo".to_string()),
            reviewers: vec![reviewer],
            interval,
            timeout,
        }
    }

    fn make_args_both(interval: u64, timeout: u64) -> WaitArgs {
        WaitArgs {
            pr: Some(1),
            repo: Some("owner/repo".to_string()),
            reviewers: vec![Reviewer::Gemini, Reviewer::Devin],
            interval,
            timeout,
        }
    }

    /// Helper to build a mock client for success cases.
    fn build_success_client(scenario: &str) -> (MockReviewClient, WaitArgs) {
        let now = Utc::now();
        match scenario {
            "already_completed_both" => (
                // Reviews already exist from both reviewers
                MockReviewClient::new()
                    .with_review(Reviewer::Gemini, now - ChronoDuration::hours(1))
                    .with_review(Reviewer::Devin, now - ChronoDuration::hours(1)),
                make_args_both(1, 5),
            ),
            "in_progress_completes_both" => (
                // Both reviews appear after polling
                MockReviewClient::new()
                    .with_comment("gemini-code-assist", "Starting review...", now)
                    .with_review(Reviewer::Gemini, now + ChronoDuration::seconds(1))
                    .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
                    .skip_first_n_review_calls(2), // Skip initial check for both
                make_args_both(1, 5),
            ),
            "one_unable_existing_other_completes" => (
                // Gemini already posted "unable", Devin review completes
                MockReviewClient::new()
                    .with_comment(
                        "gemini-code-assist",
                        "Gemini is unable to review this PR.",
                        now - ChronoDuration::hours(1),
                    )
                    .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
                    .skip_first_n_review_calls(1), // Skip initial check for Devin
                make_args_both(1, 5),
            ),
            "one_unable_during_wait_other_completes" => (
                // Gemini posts "unable" during polling, Devin review completes
                MockReviewClient::new()
                    .with_comment("gemini-code-assist", "Starting review...", now)
                    .with_comment(
                        "gemini-code-assist",
                        "Gemini is unable to review this PR.",
                        now + ChronoDuration::milliseconds(100),
                    )
                    .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
                    .skip_first_n_review_calls(1), // Skip initial check for Devin
                make_args_both(1, 5),
            ),
            "all_unable_existing" => (
                // Both reviewers already posted "unable" comments
                MockReviewClient::new()
                    .with_comment(
                        "gemini-code-assist",
                        "Gemini is unable to review this PR.",
                        now - ChronoDuration::hours(1),
                    )
                    .with_comment(
                        "devin-ai-integration",
                        "Devin is unable to review this PR.",
                        now - ChronoDuration::hours(1),
                    ),
                make_args_both(1, 5),
            ),
            "single_unable" => (
                // Single reviewer that is unable
                MockReviewClient::new().with_comment(
                    "gemini-code-assist",
                    "Gemini is unable to review this PR.",
                    now - ChronoDuration::hours(1),
                ),
                make_args_single(Reviewer::Gemini, 1, 5),
            ),
            _ => panic!("Unknown scenario: {scenario}"),
        }
    }

    #[rstest]
    #[case::already_completed_both("already_completed_both")]
    #[case::in_progress_completes_both("in_progress_completes_both")]
    #[case::one_unable_existing_other_completes("one_unable_existing_other_completes")]
    #[case::one_unable_during_wait_other_completes("one_unable_during_wait_other_completes")]
    #[case::all_unable_existing("all_unable_existing")]
    #[case::single_unable("single_unable")]
    #[tokio::test]
    async fn wait_succeeds(#[case] scenario: &str) {
        let (client, args) = build_success_client(scenario);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
    }

    /// Helper to build a mock client for error cases.
    fn build_error_client(scenario: &str) -> (MockReviewClient, WaitArgs) {
        match scenario {
            "not_started_gemini_only" => (
                MockReviewClient::new(),
                make_args_single(Reviewer::Gemini, 1, 5),
            ),
            "timeout" => (
                MockReviewClient::new(),
                make_args_both(1, 1), // 1 second timeout
            ),
            _ => panic!("Unknown scenario: {scenario}"),
        }
    }

    #[rstest]
    #[case::not_started_gemini_only("not_started_gemini_only", "Review has not started yet")]
    #[case::timeout("timeout", "Timeout waiting for review after 1 seconds")]
    #[tokio::test]
    async fn wait_fails(#[case] scenario: &str, #[case] expected_error: &str) {
        let (client, args) = build_error_client(scenario);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        let err = result.unwrap_err();
        assert_eq!(err.to_string(), expected_error);
    }

    #[rstest]
    #[case::gemini(Reviewer::Gemini)]
    #[case::devin(Reviewer::Devin)]
    #[tokio::test]
    async fn wait_detects_correct_reviewer(#[case] reviewer: Reviewer) {
        let client =
            MockReviewClient::new().with_review(reviewer, Utc::now() - ChronoDuration::hours(1));

        let args = make_args_single(reviewer, 1, 5);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wait_skips_start_detection_when_devin_included() {
        // When both Gemini and Devin are included, and Gemini has not started,
        // we should still wait because Devin doesn't require start detection
        let now = Utc::now();
        let client = MockReviewClient::new()
            // No comments from either (Gemini not started)
            // But both reviews appear after polling
            .with_review(Reviewer::Gemini, now + ChronoDuration::seconds(1))
            .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
            .skip_first_n_review_calls(2); // Skip first check for both reviewers

        let args = make_args_both(1, 5);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wait_requires_start_detection_for_gemini_only() {
        // When only Gemini is specified and it has no activity, return error
        let client = MockReviewClient::new();

        let args = make_args_single(Reviewer::Gemini, 1, 5);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;
        let err = result.unwrap_err();
        assert_eq!(err.to_string(), "Review has not started yet");
    }

    #[tokio::test]
    async fn wait_requires_all_reviewers_to_complete() {
        // When waiting for both, should wait until all complete (not just one)
        let now = Utc::now();
        let client = MockReviewClient::new()
            .with_review(Reviewer::Gemini, now + ChronoDuration::seconds(1))
            .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
            .skip_first_n_review_calls(2);

        let args = make_args_both(1, 5);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;
        assert!(result.is_ok());
    }
}
