//! Wait for an existing review from a bot reviewer.

use super::client::get_client;
use super::common::{WaitConfig, get_pr_number, get_repo_owner_and_name, wait_for_all_reviews};
use super::detector::{DetectionClient, DetectionContext, ReviewDetector};
use super::detectors::AnyDetector;
use super::error::{Result, ReviewError};
use super::reviewer::{Reviewer, builtin_default_reviewers};
use crate::shared::config::load_config;
use chrono::{DateTime, Utc};
use clap::Args;
use std::io::Write;

const POLL_INTERVAL_SECS: u64 = 15;
const TIMEOUT_SECS: u64 = 300; // 5 minutes
const START_WAIT_SECS: u64 = 30;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WaitArgs {
    /// PR number (auto-detects from current branch if not specified)
    pub pr: Option<u64>,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,

    /// Reviewers to wait for (can specify multiple, waits for all to complete).
    /// Defaults to the configured reviewer set for the repo's org, or
    /// `[devin]` if no config applies.
    #[arg(short = 'r', long = "reviewer", value_enum)]
    pub reviewers: Option<Vec<Reviewer>>,

    /// Polling interval in seconds
    #[arg(long, default_value_t = POLL_INTERVAL_SECS)]
    pub interval: u64,

    /// Timeout in seconds after a reviewer start signal is detected
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
    run_wait_with_start_timeout(
        args,
        client,
        owner,
        repo,
        pr_number,
        std::time::Duration::from_secs(START_WAIT_SECS),
    )
    .await
}

async fn run_wait_with_start_timeout<C: DetectionClient>(
    args: &WaitArgs,
    client: &C,
    owner: &str,
    repo: &str,
    pr_number: u64,
    start_timeout: std::time::Duration,
) -> Result<()> {
    let resolved = match args.reviewers.as_deref() {
        Some(r) => r.to_vec(),
        None => load_config()?
            .resolve_reviewers(owner, repo)
            .unwrap_or_else(builtin_default_reviewers),
    };
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

    // Give reviewers time to publish their first signal after a PR is created or updated.
    let active_detectors: Vec<_> = active_reviewers
        .iter()
        .map(|r| (*r, r.detector()))
        .collect();

    if !wait_for_any_started(
        &active_detectors,
        &ctx,
        start_timeout,
        std::time::Duration::from_secs(args.interval.max(1)),
    )
    .await?
    {
        println!();
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

async fn wait_for_any_started<C: DetectionClient>(
    detectors: &[(Reviewer, AnyDetector)],
    ctx: &DetectionContext<'_, C>,
    timeout: std::time::Duration,
    interval: std::time::Duration,
) -> Result<bool> {
    let started_at = tokio::time::Instant::now();

    loop {
        for (_, detector) in detectors {
            if detector.is_started(ctx).await? {
                return Ok(true);
            }
        }

        let remaining = timeout.saturating_sub(started_at.elapsed());
        if remaining.is_zero() {
            return Ok(false);
        }
        print!(
            "\rWaiting for reviewer start from {:?}... ({}s elapsed, grace: {}s)   ",
            detectors
                .iter()
                .map(|(reviewer, _)| reviewer)
                .collect::<Vec<_>>(),
            started_at.elapsed().as_secs(),
            timeout.as_secs()
        );
        std::io::stdout().flush().ok();
        tokio::time::sleep(interval.min(remaining)).await;
    }
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
            reviewers: Some(vec![Reviewer::CodeRabbit, Reviewer::Devin]),
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
                    .with_review(Reviewer::CodeRabbit, now - ChronoDuration::hours(1))
                    .with_review(Reviewer::Devin, now - ChronoDuration::hours(1)),
                make_args_both(1, 5),
            ),
            "in_progress_completes_both" => (
                // Both reviews appear after polling; CodeRabbit started (commit status present)
                MockDetectionClient::new()
                    .with_commit_status("CodeRabbit", "PENDING", now)
                    .with_review(Reviewer::CodeRabbit, now + ChronoDuration::seconds(1))
                    .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
                    .with_check_run("devin-review", None)
                    .skip_first_n_review_calls(2), // Skip initial check for both
                make_args_both(1, 5),
            ),
            "one_unable_existing_other_completes" => (
                // CodeRabbit already posted "unable", Devin review completes
                MockDetectionClient::new()
                    .with_comment(
                        "coderabbitai",
                        "## Review skipped",
                        now - ChronoDuration::hours(1),
                    )
                    .with_check_run("devin-review", None)
                    .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
                    .skip_first_n_review_calls(1), // Skip initial check for Devin
                make_args_both(1, 5),
            ),
            "one_unable_during_wait_other_completes" => (
                // CodeRabbit posts "unable" during polling, Devin review completes
                MockDetectionClient::new()
                    .with_commit_status("CodeRabbit", "PENDING", now)
                    .with_comment(
                        "coderabbitai",
                        "## Review skipped",
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
                        "coderabbitai",
                        "## Review skipped",
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
                    "coderabbitai",
                    "## Review skipped",
                    now - ChronoDuration::hours(1),
                ),
                make_args_single(Reviewer::CodeRabbit, 1, 5),
            ),
            "one_completed_other_unable" => (
                // CodeRabbit already reviewed, Devin is unable
                MockDetectionClient::new()
                    .with_review(Reviewer::CodeRabbit, now - ChronoDuration::hours(1))
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
            "not_started_coderabbit_only" => (
                // No commit status, so CodeRabbit start detection fails
                MockDetectionClient::new(),
                make_args_single(Reviewer::CodeRabbit, 1, 5),
            ),
            "neither_started_both" => {
                let now = Utc::now();
                (
                    MockDetectionClient::new()
                        .with_review(Reviewer::CodeRabbit, now + ChronoDuration::seconds(1))
                        .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
                        .skip_first_n_review_calls(2),
                    make_args_both(1, 5),
                )
            }
            "timeout" => (
                // Start signals present but no reviews ever appear
                MockDetectionClient::new()
                    .with_commit_status("CodeRabbit", "PENDING", Utc::now())
                    .with_check_run("devin-review", None),
                make_args_both(1, 1), // 1 second timeout
            ),
            _ => panic!("Unknown scenario: {scenario}"),
        }
    }

    #[rstest]
    #[case::not_started_coderabbit_only(
        "not_started_coderabbit_only",
        "Review has not started yet"
    )]
    #[case::neither_started_both("neither_started_both", "Review has not started yet")]
    #[case::timeout("timeout", "Timeout waiting for review after 1 seconds")]
    #[tokio::test]
    async fn wait_fails(#[case] scenario: &str, #[case] expected_error: &str) {
        let (client, args) = build_error_client(scenario);

        let result = if scenario == "timeout" {
            run_wait(&args, &client, "owner", "repo", 1).await
        } else {
            run_wait_with_start_timeout(
                &args,
                &client,
                "owner",
                "repo",
                1,
                std::time::Duration::ZERO,
            )
            .await
        };

        let err = result.unwrap_err();
        assert_eq!(err.to_string(), expected_error);
    }

    #[rstest]
    #[case::coderabbit(Reviewer::CodeRabbit)]
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
    async fn wait_succeeds_when_devin_check_run_present() {
        // When Devin's check run is present, start detection passes
        let now = Utc::now();
        let client = MockDetectionClient::new()
            .with_check_run("devin-review", None)
            .with_review(Reviewer::CodeRabbit, now + ChronoDuration::seconds(1))
            .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
            .skip_first_n_review_calls(2);

        let args = make_args_both(1, 5);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wait_requires_all_reviewers_to_complete() {
        // When waiting for both, should wait until all complete (not just one)
        let now = Utc::now();
        let client = MockDetectionClient::new()
            .with_commit_status("CodeRabbit", "PENDING", now)
            .with_check_run("devin-review", None)
            .with_review(Reviewer::CodeRabbit, now + ChronoDuration::seconds(1))
            .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
            .skip_first_n_review_calls(2);

        let args = make_args_both(1, 5);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wait_recognizes_devin_commit_status_as_started() {
        let client =
            MockDetectionClient::new().with_commit_status("Devin Review", "PENDING", Utc::now());
        let args = make_args_single(Reviewer::Devin, 1, 0);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        assert_eq!(
            result.unwrap_err().to_string(),
            "Timeout waiting for review after 0 seconds"
        );
    }

    #[tokio::test]
    async fn wait_waits_for_devin_status_to_succeed() {
        let now = Utc::now();
        let client = MockDetectionClient::new()
            .with_commit_status("Devin Review", "SUCCESS", now + ChronoDuration::seconds(1))
            .delay_status_success(2);
        let args = make_args_single(Reviewer::Devin, 1, 5);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wait_accepts_trial_skipped_success_status_without_review() {
        let client =
            MockDetectionClient::new().with_commit_status("Devin Review", "SUCCESS", Utc::now());
        let args = make_args_single(Reviewer::Devin, 1, 5);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wait_does_not_accept_devin_review_for_old_head() {
        let client = MockDetectionClient::new()
            .with_head_oid("new-head")
            .with_review_on_commit(
                Reviewer::Devin,
                Utc::now() - ChronoDuration::hours(1),
                "old-head",
            )
            .with_commit_status("Devin Review", "PENDING", Utc::now());
        let args = make_args_single(Reviewer::Devin, 1, 0);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        assert_eq!(
            result.unwrap_err().to_string(),
            "Timeout waiting for review after 0 seconds"
        );
    }

    #[tokio::test]
    async fn wait_returns_not_started_after_start_grace() {
        let client = MockDetectionClient::new();
        let args = make_args_single(Reviewer::Devin, 1, 5);

        let result = run_wait_with_start_timeout(
            &args,
            &client,
            "owner",
            "repo",
            1,
            std::time::Duration::from_millis(20),
        )
        .await;

        assert_eq!(
            result.unwrap_err().to_string(),
            "Review has not started yet"
        );
    }

    #[tokio::test]
    async fn wait_detects_devin_start_signal_that_appears_during_grace() {
        let client = MockDetectionClient::new()
            .with_commit_status("Devin Review", "PENDING", Utc::now())
            .delay_start_signal(1);
        let ctx = DetectionContext {
            github: &client,
            owner: "owner",
            repo: "repo",
            pr: 1,
        };
        let detectors = vec![(Reviewer::Devin, Reviewer::Devin.detector())];

        let started = wait_for_any_started(
            &detectors,
            &ctx,
            std::time::Duration::from_millis(50),
            std::time::Duration::from_millis(2),
        )
        .await
        .unwrap();

        assert!(started);
    }

    #[tokio::test]
    async fn wait_accepts_completed_devin_check_run_without_review_or_status() {
        let client = MockDetectionClient::new().with_check_run(
            "devin-review",
            Some(Utc::now() + ChronoDuration::seconds(1)),
        );
        let args = make_args_single(Reviewer::Devin, 1, 5);

        let result = run_wait(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
    }
}
