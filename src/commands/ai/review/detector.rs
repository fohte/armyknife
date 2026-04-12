//! Review detection trait and detection method enums.
//!
//! Provides a strategy-based abstraction for detecting bot review start/completion,
//! with declarative detection method enums that a generic polling engine can interpret.

use std::future::Future;

use chrono::{DateTime, Utc};

use super::error::Result;

/// How to detect that a reviewer has started working on a PR.
pub enum StartDetection {
    /// The bot reacted to the PR body with a specific emoji (e.g., Gemini uses :eyes:).
    BodyReaction { emoji: &'static str },
    /// A GitHub check run with the given name has appeared.
    CheckRun { name: &'static str },
}

/// How to detect that a reviewer has completed its review.
pub enum CompletionDetection {
    /// A PR review (via GitHub review API) has been submitted by the bot.
    ReviewSubmitted,
    /// A PR review has been submitted, OR a check run with the given name has completed.
    /// Whichever signal arrives first counts as completion.
    ReviewOrCheckRun { check_name: &'static str },
}

/// Context passed to detection methods, carrying the GitHub API client
/// and PR identification info.
pub struct DetectionContext<'a, C: DetectionClient> {
    pub github: &'a C,
    pub owner: &'a str,
    pub repo: &'a str,
    pub pr: u64,
}

/// Low-level GitHub API operations needed for review detection.
///
/// Extracted from the higher-level review workflow so that detectors
/// can be tested with mock implementations.
pub trait DetectionClient: Send + Sync {
    /// Find the latest review timestamp from a bot identified by `bot_login`.
    fn find_latest_review(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        bot_login: &str,
    ) -> impl Future<Output = Result<Option<DateTime<Utc>>>> + Send;

    /// Check if the bot posted an "unable to review" comment after `after`.
    fn find_unable_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        bot_login: &str,
        unable_marker: &str,
        after: DateTime<Utc>,
    ) -> impl Future<Output = Result<Option<String>>> + Send;

    /// Get the latest commit timestamp on the PR.
    fn get_latest_commit_time(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> impl Future<Output = Result<Option<DateTime<Utc>>>> + Send;

    /// Post a comment on the PR.
    fn post_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        body: &str,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Check whether the PR body has a reaction with the given emoji.
    fn has_body_reaction(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        emoji: &str,
    ) -> impl Future<Output = Result<bool>> + Send;

    /// Check whether a check run with the given name exists on the PR's head commit.
    fn has_check_run(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        check_name: &str,
    ) -> impl Future<Output = Result<bool>> + Send;

    /// Get the completion timestamp of a check run, if it has completed.
    fn check_run_completed_at(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        check_name: &str,
    ) -> impl Future<Output = Result<Option<DateTime<Utc>>>> + Send;
}

/// Strategy trait for detecting bot review start and completion.
///
/// Each bot reviewer implements this trait. The default implementations of
/// `is_started` and `is_completed` interpret the declarative enums returned
/// by `start_method` / `completion_method`. Override them only when a bot
/// needs detection logic that cannot be expressed declaratively.
pub trait ReviewDetector: Send + Sync {
    /// The bot's GitHub login name (e.g., "gemini-code-assist").
    fn bot_login(&self) -> &'static str;

    /// Command to trigger a review (e.g., "/gemini review"). None if not supported.
    fn review_command(&self) -> Option<&'static str> {
        Option::None
    }

    /// Marker text in comments that indicates the bot is unable to review.
    fn unable_marker(&self) -> &'static str;

    /// Declarative description of how to detect review start.
    fn start_method(&self) -> StartDetection;

    /// Declarative description of how to detect review completion.
    fn completion_method(&self) -> CompletionDetection;

    /// Check whether the reviewer has started working on the PR.
    ///
    /// Default implementation interprets `start_method()`.
    async fn is_started<C: DetectionClient>(&self, ctx: &DetectionContext<'_, C>) -> Result<bool> {
        match self.start_method() {
            StartDetection::BodyReaction { emoji } => {
                ctx.github
                    .has_body_reaction(ctx.owner, ctx.repo, ctx.pr, emoji)
                    .await
            }
            StartDetection::CheckRun { name } => {
                ctx.github
                    .has_check_run(ctx.owner, ctx.repo, ctx.pr, name)
                    .await
            }
        }
    }

    /// Check whether the reviewer has completed its review.
    ///
    /// Returns the completion timestamp if completed, or None if still pending.
    /// Default implementation interprets `completion_method()`.
    async fn is_completed<C: DetectionClient>(
        &self,
        ctx: &DetectionContext<'_, C>,
    ) -> Result<Option<DateTime<Utc>>> {
        match self.completion_method() {
            CompletionDetection::ReviewSubmitted => {
                ctx.github
                    .find_latest_review(ctx.owner, ctx.repo, ctx.pr, self.bot_login())
                    .await
            }
            CompletionDetection::ReviewOrCheckRun { check_name } => {
                if let Some(t) = ctx
                    .github
                    .find_latest_review(ctx.owner, ctx.repo, ctx.pr, self.bot_login())
                    .await?
                {
                    return Ok(Some(t));
                }
                ctx.github
                    .check_run_completed_at(ctx.owner, ctx.repo, ctx.pr, check_name)
                    .await
            }
        }
    }

    /// Check if the bot posted an "unable to review" comment after `after`.
    async fn find_unable_comment<C: DetectionClient>(
        &self,
        ctx: &DetectionContext<'_, C>,
        after: DateTime<Utc>,
    ) -> Result<Option<String>> {
        ctx.github
            .find_unable_comment(
                ctx.owner,
                ctx.repo,
                ctx.pr,
                self.bot_login(),
                self.unable_marker(),
                after,
            )
            .await
    }

    /// Post a review request comment. Returns error if not supported.
    async fn request_review<C: DetectionClient>(
        &self,
        ctx: &DetectionContext<'_, C>,
    ) -> Result<()> {
        match self.review_command() {
            Some(cmd) => {
                ctx.github
                    .post_comment(ctx.owner, ctx.repo, ctx.pr, cmd)
                    .await
            }
            Option::None => Err(super::error::ReviewError::RequestNotSupported(
                self.bot_login().to_string(),
            )
            .into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ai::review::client::mock::MockDetectionClient;

    /// A test-only detector with no review command support.
    struct NoCommandDetector;

    impl ReviewDetector for NoCommandDetector {
        fn bot_login(&self) -> &'static str {
            "test-bot"
        }

        fn unable_marker(&self) -> &'static str {
            "test-bot is unable to"
        }

        fn start_method(&self) -> StartDetection {
            StartDetection::CheckRun { name: "test-bot" }
        }

        fn completion_method(&self) -> CompletionDetection {
            CompletionDetection::ReviewSubmitted
        }
    }

    #[tokio::test]
    async fn request_review_returns_error_when_no_command() {
        let client = MockDetectionClient::new();
        let ctx = DetectionContext {
            github: &client,
            owner: "owner",
            repo: "repo",
            pr: 1,
        };

        let result = NoCommandDetector.request_review(&ctx).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("does not support request command")
        );
    }
}
