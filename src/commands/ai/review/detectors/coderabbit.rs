//! CodeRabbit review detector.

use crate::commands::ai::review::detector::{CompletionDetection, ReviewDetector, StartDetection};

pub struct CodeRabbitDetector;

impl ReviewDetector for CodeRabbitDetector {
    fn bot_login(&self) -> &'static str {
        // GitHub GraphQL `author.login` returns "coderabbitai" (REST adds the "[bot]" suffix).
        "coderabbitai"
    }

    fn review_command(&self) -> Option<&'static str> {
        Some("@coderabbitai review")
    }

    fn unable_marker(&self) -> &'static str {
        // CodeRabbit posts an issue comment containing "## Review skipped" when it declines to review.
        "Review skipped"
    }

    fn start_method(&self) -> StartDetection {
        // CodeRabbit signals review start via the legacy Commit Status API,
        // not Check Runs (its check_suite stays queued).
        StartDetection::CommitStatus {
            context: "CodeRabbit",
        }
    }

    fn completion_method(&self) -> CompletionDetection {
        CompletionDetection::ReviewOrCommitStatus {
            context: "CodeRabbit",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ai::review::client::mock::MockDetectionClient;
    use crate::commands::ai::review::detector::DetectionContext;
    use crate::commands::ai::review::reviewer::Reviewer;
    use chrono::DateTime;

    fn ctx(client: &MockDetectionClient) -> DetectionContext<'_, MockDetectionClient> {
        DetectionContext {
            github: client,
            owner: "owner",
            repo: "repo",
            pr: 1,
        }
    }

    #[tokio::test]
    async fn is_started_returns_true_when_commit_status_present() {
        let t = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .to_utc();
        let client = MockDetectionClient::new().with_commit_status("CodeRabbit", "PENDING", t);

        let started = CodeRabbitDetector.is_started(&ctx(&client)).await.unwrap();

        assert!(started);
    }

    #[tokio::test]
    async fn is_started_returns_false_without_status() {
        let client = MockDetectionClient::new();

        let started = CodeRabbitDetector.is_started(&ctx(&client)).await.unwrap();

        assert!(!started);
    }

    #[tokio::test]
    async fn is_completed_via_commit_status_success() {
        let t = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .to_utc();
        let client = MockDetectionClient::new().with_commit_status("CodeRabbit", "SUCCESS", t);

        let completed = CodeRabbitDetector
            .is_completed(&ctx(&client))
            .await
            .unwrap();

        assert_eq!(completed, Some(t));
    }

    #[tokio::test]
    async fn is_completed_via_review_submission() {
        let t = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .to_utc();
        let client = MockDetectionClient::new().with_review(Reviewer::CodeRabbit, t);

        let completed = CodeRabbitDetector
            .is_completed(&ctx(&client))
            .await
            .unwrap();

        assert_eq!(completed, Some(t));
    }

    #[tokio::test]
    async fn is_completed_returns_none_when_only_pending() {
        let t = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .to_utc();
        let client = MockDetectionClient::new().with_commit_status("CodeRabbit", "PENDING", t);

        let completed = CodeRabbitDetector
            .is_completed(&ctx(&client))
            .await
            .unwrap();

        assert!(completed.is_none());
    }
}
