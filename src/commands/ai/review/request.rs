//! Request a review from a bot reviewer.

use super::client::get_client;
use super::common::{WaitConfig, get_pr_number, get_repo_owner_and_name, wait_for_all_reviews};
use super::detector::{DetectionClient, DetectionContext, ReviewDetector};
use super::error::Result;
use super::reviewer::{Reviewer, builtin_default_reviewers};
use crate::shared::config::load_config;
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

    /// Reviewers to request (can specify multiple, waits for all to complete).
    /// Reviewers that don't support request (like Devin) will be waited for without requesting.
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

pub async fn run(args: &RequestArgs) -> Result<()> {
    run_with_client(args, &get_client()).await
}

pub async fn run_with_client<C: DetectionClient>(args: &RequestArgs, client: &C) -> Result<()> {
    let (owner, repo) = get_repo_owner_and_name(args.repo.as_deref())?;
    let pr_number = get_pr_number(&owner, &repo, args.pr).await?;

    run_request(args, client, &owner, &repo, pr_number).await
}

/// Internal implementation that can be tested without PR/repo detection.
pub(crate) async fn run_request<C: DetectionClient>(
    args: &RequestArgs,
    client: &C,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> Result<()> {
    let resolved = match args.reviewers.as_deref() {
        Some(r) => r.to_vec(),
        None => load_config()?
            .resolve_reviewers(owner, repo)
            .unwrap_or_else(builtin_default_reviewers),
    };
    let reviewers = &resolved;
    println!("Checking PR #{pr_number} for {:?} review(s)...", reviewers);

    // Record start time to detect "new" reviews
    let start_time = Utc::now();

    let ctx = DetectionContext {
        github: client,
        owner,
        repo,
        pr: pr_number,
    };

    // Build detectors and fetch review times for all reviewers once
    let detectors: Vec<_> = reviewers.iter().map(|r| (*r, r.detector())).collect();
    let mut review_times = Vec::new();
    for (reviewer, detector) in &detectors {
        let review_time = detector.is_completed(&ctx).await?;
        review_times.push((*reviewer, review_time));
    }

    // Fetch latest commit time once
    let latest_commit_time = client
        .get_latest_commit_time(owner, repo, pr_number)
        .await?;

    // Check if any reviewer has an up-to-date review
    if let Some(commit_time) = latest_commit_time {
        for (reviewer, review_time) in &review_times {
            if let Some(rt) = review_time
                && commit_time <= *rt
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

    // Request re-review from reviewers that support it and have old reviews
    for ((reviewer, _review_time), (_, detector)) in review_times.iter().zip(detectors.iter()) {
        if detector.review_command().is_some() && _review_time.is_some() {
            // New commits exist (we already checked above that no up-to-date review exists),
            // so request re-review
            println!("Posting {:?} review command...", reviewer);
            detector.request_review(&ctx).await?;
        }
    }

    // Log which reviewers we're waiting for
    let non_requestable: Vec<_> = detectors
        .iter()
        .filter(|(_, d)| d.review_command().is_none())
        .map(|(r, _)| r)
        .collect();
    if !non_requestable.is_empty() {
        println!(
            "Waiting for {:?} (cannot request, will wait for automatic review)...",
            non_requestable
        );
    }

    println!(
        "Waiting for all reviews to complete from {:?}...",
        reviewers
    );

    // Poll for new reviews from all reviewers
    let config = WaitConfig {
        reviewers: reviewers.clone(),
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

    fn make_args_both(interval: u64, timeout: u64) -> RequestArgs {
        RequestArgs {
            pr: Some(1),
            repo: Some("owner/repo".to_string()),
            reviewers: Some(vec![Reviewer::Gemini, Reviewer::Devin]),
            interval,
            timeout,
        }
    }

    /// Helper to build a mock client for success cases.
    fn build_success_client(scenario: &str) -> MockDetectionClient {
        let now = Utc::now();
        match scenario {
            "first_review_both" => {
                // No existing reviews, both reviews appear after polling
                // Skip 2 calls: initial check for both reviewers
                MockDetectionClient::new()
                    .with_review(Reviewer::Gemini, now + ChronoDuration::milliseconds(100))
                    .with_review(Reviewer::Devin, now + ChronoDuration::milliseconds(100))
                    .skip_first_n_review_calls(2)
            }
            "already_reviewed_both" => {
                // Both reviews exist, commit older than reviews -> skip
                MockDetectionClient::new()
                    .with_review(Reviewer::Gemini, now - ChronoDuration::hours(1))
                    .with_review(Reviewer::Devin, now - ChronoDuration::hours(1))
                    .with_latest_commit_time(now - ChronoDuration::hours(2))
            }
            _ => panic!("Unknown scenario: {scenario}"),
        }
    }

    #[rstest]
    #[case::first_review_both("first_review_both")]
    #[case::already_reviewed_both("already_reviewed_both")]
    #[tokio::test]
    async fn request_succeeds(#[case] scenario: &str) {
        let client = build_success_client(scenario);
        let args = make_args_both(1, 5);

        let result = run_request(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
    }

    #[tokio::test]
    async fn request_fails_on_timeout() {
        let client = MockDetectionClient::new();
        let args = make_args_both(1, 1);
        let result = run_request(&args, &client, "owner", "repo", 1).await;

        let err_msg = result.unwrap_err().to_string();
        assert_eq!(err_msg, "Timeout waiting for review after 1 seconds");
    }

    #[tokio::test]
    async fn request_succeeds_when_all_unable() {
        let now = Utc::now();
        let client = MockDetectionClient::new()
            .with_comment(
                "gemini-code-assist",
                "Gemini is unable to review this PR.",
                now + ChronoDuration::milliseconds(100),
            )
            .with_comment(
                "devin-ai-integration",
                "Devin is unable to review this PR.",
                now + ChronoDuration::milliseconds(100),
            );

        let args = make_args_both(1, 5);
        let result = run_request(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn request_skips_unable_reviewer_and_waits_for_other() {
        // Gemini posts "unable" during polling, but Devin completes review
        let now = Utc::now();
        let client = MockDetectionClient::new()
            .with_comment(
                "gemini-code-assist",
                "Gemini is unable to review this PR.",
                now + ChronoDuration::milliseconds(100),
            )
            .with_review(Reviewer::Devin, now + ChronoDuration::milliseconds(100))
            .skip_first_n_review_calls(2); // Skip initial check for both reviewers

        let args = make_args_both(1, 5);
        let result = run_request(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn request_posts_gemini_command_only_when_rereview_needed() {
        // When Gemini has an old review and new commits exist,
        // only post command for Gemini (Devin doesn't support command-based requests)
        let now = Utc::now();
        let client = MockDetectionClient::new()
            // Only Gemini has an old review
            .with_review(Reviewer::Gemini, now - ChronoDuration::hours(2))
            // New reviews appear after polling (both reviewers)
            .with_review(Reviewer::Gemini, now + ChronoDuration::seconds(1))
            .with_review(Reviewer::Devin, now + ChronoDuration::seconds(1))
            // New commit after old review
            .with_latest_commit_time(now - ChronoDuration::hours(1))
            // Apply cutoff to initial round (first 2 calls: one for each reviewer)
            .with_initial_review_cutoff(now, 2);

        let args = make_args_both(1, 5);

        let result = run_request(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
        let posted = client.posted_comments.lock().unwrap();
        // Only Gemini gets a comment (it has an old review and supports command-based requests)
        assert_eq!(posted.len(), 1);
        assert_eq!(posted[0].body, "/gemini review");
    }

    #[tokio::test]
    async fn request_waits_for_both_reviewers() {
        // Both reviewers must complete for success (not just one)
        let now = Utc::now();
        let client = MockDetectionClient::new()
            .with_review(Reviewer::Gemini, now + ChronoDuration::milliseconds(100))
            .with_review(Reviewer::Devin, now + ChronoDuration::milliseconds(100))
            .skip_first_n_review_calls(2); // Skip initial check for both reviewers

        let args = make_args_both(1, 5);

        let result = run_request(&args, &client, "owner", "repo", 1).await;

        assert!(result.is_ok());
        let posted = client.posted_comments.lock().unwrap();
        assert!(posted.is_empty()); // No comments posted (no existing reviews to re-request)
    }
}
