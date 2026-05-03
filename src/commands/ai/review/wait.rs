//! Wait for an existing review from a bot reviewer.

use super::client::get_client;
use super::common::{WaitConfig, get_pr_number, get_repo_owner_and_name, wait_for_all_reviews};
use super::detector::{DetectionClient, DetectionContext, ReviewDetector};
use super::error::{Result, ReviewError};
use super::reviewer::{Reviewer, resolve_reviewers_with_default};
use crate::shared::config::Config;
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

    /// Reviewers to wait for (can specify multiple, waits for all to complete).
    /// Defaults to the configured reviewer set for the repo's org, or
    /// `[gemini, devin]` if no config applies.
    #[arg(short = 'r', long = "reviewer", value_enum)]
    pub reviewers: Option<Vec<Reviewer>>,

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

pub async fn run_with_client<C: DetectionClient>(args: &WaitArgs, client: &C) -> Result<()> {
    let (owner, repo) = get_repo_owner_and_name(args.repo.as_deref())?;
    let pr_number = get_pr_number(&owner, &repo, args.pr).await?;

    run_wait(args, client, &owner, &repo, pr_number).await
}

/// Internal implementation that can be tested without PR/repo detection.
pub(crate) async fn run_wait<C: DetectionClient>(
    args: &WaitArgs,
    client: &C,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> Result<()> {
    let config = Config::load_or_default();
    let resolved = resolve_reviewers_with_default(args.reviewers.as_deref(), &config, owner, repo);
    let reviewers = &resolved;
    println!("Checking PR #{pr_number} for {:?} review(s)...", reviewers);

    let ctx = DetectionContext {
        github: client,
        owner,
        repo,
        pr: pr_number,
    };

    // Record start time to detect "new" reviews
    let start_time = Utc::now();

    // Build detectors
    let detectors: Vec<_> = reviewers.iter().map(|r| (*r, r.detector())).collect();

    // Check if reviewers have already posted reviews
    let mut already_completed: Vec<Reviewer> = Vec::new();
    for (reviewer, detector) in &detectors {
        if let Some(review_time) = detector.is_completed(&ctx).await? {
            println!(
                "{:?} review already completed at {}",
                reviewer,
                review_time.format("%Y-%m-%d %H:%M:%S UTC")
            );
            already_completed.push(*reviewer);
        }
    }
    if already_completed.len() == reviewers.len() {
        println!("All reviews already completed!");
        return Ok(());
    }

    // Check if any reviewer already posted an "unable to" comment.
    // Skip unable reviewers instead of erroring.
    let past_time = DateTime::<Utc>::MIN_UTC;
    let mut skipped_reviewers: Vec<Reviewer> = Vec::new();
    for (reviewer, detector) in &detectors {
        if already_completed.contains(reviewer) {
            continue;
        }
        if let Some(unable_msg) = detector.find_unable_comment(&ctx, past_time).await? {
            println!(
                "{:?} is unable to review this PR: {}. Skipping.",
                reviewer, unable_msg
            );
            skipped_reviewers.push(*reviewer);
        }
    }

    // Reviewers that still need to post a new review
    let active_reviewers: Vec<Reviewer> = reviewers
        .iter()
        .filter(|r| !already_completed.contains(r) && !skipped_reviewers.contains(r))
        .copied()
        .collect();
    if active_reviewers.is_empty() {
        println!("All reviews already completed or skipped!");
        return Ok(());
    }

    // Check if any reviewer has started. If none have started, return error.
    let active_detectors: Vec<_> = active_reviewers
        .iter()
        .map(|r| (*r, r.detector()))
        .collect();

    let mut any_started = false;
    for (_, detector) in &active_detectors {
        if detector.is_started(&ctx).await? {
            any_started = true;
            break;
        }
    }

    if !any_started {
        return Err(ReviewError::ReviewNotStarted.into());
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
    use crate::commands::ai::review::client::mock::MockDetectionClient;
    use chrono::Duration as ChronoDuration;
    use rstest::rstest;

    fn make_args_single(reviewer: Reviewer, interval: u64, timeout: u64) -> WaitArgs {
        WaitArgs {
            pr: Some(1),
            repo: Some("owner/repo".to_string()),
            reviewers: Some(vec![reviewer]),
            interval,
            timeout,
        }
    }

    fn make_args_both(interval: u64, timeout: u64) -> WaitArgs {
        WaitArgs {
            pr: Some(1),
            repo: Some("owner/repo".to_string()),
            reviewers: Some(vec![Reviewer::Gemini, Reviewer::Devin]),
            interval,
            timeout,
        }
    }

    /// Helper to build a mock client for success cases.
    fn build_success_client(scenario: &str) -> (MockDetectionClient, WaitArgs) {
        let now = Utc::now();
        match scenario {
            "already_completed_both" => (
                // Reviews already exist from both reviewers
                MockDetectionClient::new()
                    .with_review(Reviewer::Gemini, now - ChronoDuration::hours(1))
                    .with_review(Reviewer::Devin, now - ChronoDuration::hours(1)),
                make_args_both(1, 5),
            ),
            "in_progress_completes_both" => (
                // Both reviews appear after polling; Gemini started (reaction present)
                MockDetectionClient::new()
                    .with_reaction("EYES")
                    .with_review(Reviewer::Gemini, now + ChronoDuration::seconds(1))
                    .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
                    .with_check_run("devin-review", None)
                    .skip_first_n_review_calls(2), // Skip initial check for both
                make_args_both(1, 5),
            ),
            "one_unable_existing_other_completes" => (
                // Gemini already posted "unable", Devin review completes
                MockDetectionClient::new()
                    .with_comment(
                        "gemini-code-assist",
                        "Gemini is unable to review this PR.",
                        now - ChronoDuration::hours(1),
                    )
                    .with_check_run("devin-review", None)
                    .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
                    .skip_first_n_review_calls(1), // Skip initial check for Devin
                make_args_both(1, 5),
            ),
            "one_unable_during_wait_other_completes" => (
                // Gemini posts "unable" during polling, Devin review completes
                MockDetectionClient::new()
                    .with_reaction("EYES")
                    .with_comment(
                        "gemini-code-assist",
                        "Gemini is unable to review this PR.",
                        now + ChronoDuration::milliseconds(100),
                    )
                    .with_check_run("devin-review", None)
                    .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
                    .skip_first_n_review_calls(1), // Skip initial check for Devin
                make_args_both(1, 5),
            ),
            "all_unable_existing" => (
                // Both reviewers already posted "unable" comments
                MockDetectionClient::new()
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
                MockDetectionClient::new().with_comment(
                    "gemini-code-assist",
                    "Gemini is unable to review this PR.",
                    now - ChronoDuration::hours(1),
                ),
                make_args_single(Reviewer::Gemini, 1, 5),
            ),
            "one_completed_other_unable" => (
                // Gemini already reviewed, Devin is unable
                MockDetectionClient::new()
                    .with_review(Reviewer::Gemini, now - ChronoDuration::hours(1))
                    .with_comment(
                        "devin-ai-integration",
                        "Devin is unable to review this PR.",
                        now - ChronoDuration::hours(1),
                    ),
                make_args_both(1, 5),
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
    #[case::one_completed_other_unable("one_completed_other_unable")]
    #[tokio::test]
    async fn wait_succeeds(#[case] scenario: &str) {
        let (client, args) = build_success_client(scenario);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
    }

    /// Helper to build a mock client for error cases.
    fn build_error_client(scenario: &str) -> (MockDetectionClient, WaitArgs) {
        match scenario {
            "not_started_gemini_only" => (
                // No reactions, so Gemini start detection fails
                MockDetectionClient::new(),
                make_args_single(Reviewer::Gemini, 1, 5),
            ),
            "timeout" => (
                // Start signals present but no reviews ever appear
                MockDetectionClient::new()
                    .with_reaction("EYES")
                    .with_check_run("devin-review", None),
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
            MockDetectionClient::new().with_review(reviewer, Utc::now() - ChronoDuration::hours(1));

        let args = make_args_single(reviewer, 1, 5);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wait_skips_start_detection_when_devin_included() {
        // When both Gemini and Devin are included, and Gemini has not started,
        // we should still wait because Devin doesn't require start detection
        // (Devin uses CheckRun which also returns false here, but the mix
        // of start detection types means we don't fail)
        let now = Utc::now();
        let client = MockDetectionClient::new()
            .with_review(Reviewer::Gemini, now + ChronoDuration::seconds(1))
            .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
            .skip_first_n_review_calls(2); // Skip first check for both reviewers

        let args = make_args_both(1, 5);

        // Both Gemini and Devin require start detection (BodyReaction and CheckRun),
        // but neither has started. However, the wait loop still works because
        // the reviews appear during polling.
        // In the new detector model, both have non-None start detection, so if neither
        // has started, ReviewNotStarted is returned. This is the correct behavior since
        // we should only wait when at least one reviewer has started.
        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        // With the new detector model, both reviewers require start detection,
        // so if neither has started, we get an error.
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Review has not started yet"
        );
    }

    #[tokio::test]
    async fn wait_succeeds_when_devin_check_run_present() {
        // When Devin's check run is present, start detection passes
        let now = Utc::now();
        let client = MockDetectionClient::new()
            .with_check_run("devin-review", None)
            .with_reaction("EYES")
            .with_review(Reviewer::Gemini, now + ChronoDuration::seconds(1))
            .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
            .skip_first_n_review_calls(2);

        let args = make_args_both(1, 5);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wait_requires_start_detection_for_gemini_only() {
        // When only Gemini is specified and no reaction is present, return error
        let client = MockDetectionClient::new();

        let args = make_args_single(Reviewer::Gemini, 1, 5);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;
        let err = result.unwrap_err();
        assert_eq!(err.to_string(), "Review has not started yet");
    }

    #[tokio::test]
    async fn wait_requires_all_reviewers_to_complete() {
        // When waiting for both, should wait until all complete (not just one)
        let now = Utc::now();
        let client = MockDetectionClient::new()
            .with_reaction("EYES")
            .with_check_run("devin-review", None)
            .with_review(Reviewer::Gemini, now + ChronoDuration::seconds(1))
            .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
            .skip_first_n_review_calls(2);

        let args = make_args_both(1, 5);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;
        assert!(result.is_ok());
    }
}
