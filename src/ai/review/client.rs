//! Review client trait and implementations.

use super::error::{Result, ReviewError};
use super::reviewer::Reviewer;
use crate::gh::check_pr_review::fetch_pr_data;
use crate::github::OctocrabClient;
use chrono::{DateTime, Utc};
use indoc::indoc;
use serde::Deserialize;
use serde_json::json;

/// Trait for review-related GitHub API operations.
#[async_trait::async_trait]
pub trait ReviewClient: Send + Sync {
    /// Find the latest review timestamp from the specified reviewer.
    async fn find_latest_review(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        reviewer: Reviewer,
    ) -> Result<Option<DateTime<Utc>>>;

    /// Check if the reviewer posted an "unable to" comment after start_time.
    async fn check_reviewer_unable_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        start_time: DateTime<Utc>,
        reviewer: Reviewer,
    ) -> Result<Option<String>>;

    /// Check if the reviewer has any activity (comments) on the PR.
    async fn has_reviewer_activity(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        reviewer: Reviewer,
    ) -> Result<bool>;

    /// Get the latest commit timestamp on the PR.
    async fn get_latest_commit_time(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<Option<DateTime<Utc>>>;

    /// Post a review request comment.
    async fn post_review_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        reviewer: Reviewer,
    ) -> Result<()>;
}

/// Production implementation using octocrab.
pub struct OctocrabReviewClient;

// GraphQL query for PR comments and commits
const PR_INFO_QUERY: &str = indoc! {"
    query($owner: String!, $repo: String!, $pr: Int!) {
        repository(owner: $owner, name: $repo) {
            pullRequest(number: $pr) {
                comments(first: 100) {
                    nodes {
                        author { login }
                        body
                        createdAt
                    }
                }
                commits(last: 1) {
                    nodes {
                        commit {
                            committedDate
                        }
                    }
                }
            }
        }
    }
"};

#[derive(Debug, Deserialize)]
struct PrInfoResponse {
    data: Option<PrInfoData>,
}

#[derive(Debug, Deserialize)]
struct PrInfoData {
    repository: Option<PrInfoRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrInfoRepository {
    pull_request: Option<PrInfoPullRequest>,
}

#[derive(Debug, Deserialize)]
struct PrInfoPullRequest {
    comments: PrInfoComments,
    commits: PrInfoCommits,
}

#[derive(Debug, Deserialize)]
struct PrInfoComments {
    nodes: Vec<PrInfoComment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrInfoComment {
    pub author: Option<PrInfoAuthor>,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct PrInfoAuthor {
    pub login: String,
}

#[derive(Debug, Deserialize)]
struct PrInfoCommits {
    nodes: Vec<PrInfoCommitNode>,
}

#[derive(Debug, Deserialize)]
struct PrInfoCommitNode {
    commit: PrInfoCommit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrInfoCommit {
    committed_date: String,
}

impl OctocrabReviewClient {
    async fn fetch_pr_info(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<PrInfoPullRequest> {
        let client = OctocrabClient::get()?;
        let variables = json!({
            "owner": owner,
            "repo": repo,
            "pr": pr_number,
        });

        let response: PrInfoResponse = client.graphql(PR_INFO_QUERY, variables).await?;

        response
            .data
            .and_then(|d| d.repository)
            .and_then(|r| r.pull_request)
            .ok_or_else(|| ReviewError::RepoInfoError("Pull request not found".to_string()))
    }
}

#[async_trait::async_trait]
impl ReviewClient for OctocrabReviewClient {
    async fn find_latest_review(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        reviewer: Reviewer,
    ) -> Result<Option<DateTime<Utc>>> {
        let pr_data = fetch_pr_data(owner, repo, pr_number, true)
            .await
            .map_err(|e| ReviewError::RepoInfoError(e.to_string()))?;

        let bot_login = reviewer.bot_login();
        let mut latest: Option<DateTime<Utc>> = None;

        for review in &pr_data.reviews {
            if let Some(author) = &review.author
                && author.login == bot_login
            {
                let created_at: DateTime<Utc> = review
                    .created_at
                    .parse()
                    .map_err(|_| ReviewError::TimestampParseError(review.created_at.clone()))?;

                latest = Some(match latest {
                    Some(prev) if created_at > prev => created_at,
                    Some(prev) => prev,
                    None => created_at,
                });
            }
        }

        Ok(latest)
    }

    async fn check_reviewer_unable_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        start_time: DateTime<Utc>,
        reviewer: Reviewer,
    ) -> Result<Option<String>> {
        let bot_login = reviewer.bot_login();
        let unable_marker = reviewer.unable_marker();

        let pr_info = self.fetch_pr_info(owner, repo, pr_number).await?;

        for comment in &pr_info.comments.nodes {
            if let Some(author) = &comment.author
                && author.login == bot_login
                && comment.body.contains(unable_marker)
                && let Ok(created_at) = comment.created_at.parse::<DateTime<Utc>>()
                && created_at > start_time
            {
                return Ok(Some(comment.body.clone()));
            }
        }

        Ok(None)
    }

    async fn has_reviewer_activity(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        reviewer: Reviewer,
    ) -> Result<bool> {
        let bot_login = reviewer.bot_login();
        let pr_info = self.fetch_pr_info(owner, repo, pr_number).await?;

        Ok(pr_info
            .comments
            .nodes
            .iter()
            .any(|c| c.author.as_ref().is_some_and(|a| a.login == bot_login)))
    }

    async fn get_latest_commit_time(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<Option<DateTime<Utc>>> {
        let pr_info = self.fetch_pr_info(owner, repo, pr_number).await?;

        let Some(commit_node) = pr_info.commits.nodes.first() else {
            return Ok(None);
        };

        commit_node
            .commit
            .committed_date
            .parse::<DateTime<Utc>>()
            .map(Some)
            .map_err(|_| {
                ReviewError::TimestampParseError(commit_node.commit.committed_date.clone())
            })
    }

    async fn post_review_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        reviewer: Reviewer,
    ) -> Result<()> {
        let review_command = reviewer.review_command();
        let client = OctocrabClient::get()?;

        client
            .client
            .issues(owner, repo)
            .create_comment(pr_number, review_command)
            .await
            .map_err(|e| ReviewError::CommentError(format!("Failed to post comment: {e}")))?;

        Ok(())
    }
}

/// Get the default production client.
pub fn get_client() -> OctocrabReviewClient {
    OctocrabReviewClient
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Mock data for a PR review.
    #[derive(Clone, Debug)]
    pub struct MockReview {
        pub reviewer: Reviewer,
        pub created_at: DateTime<Utc>,
    }

    /// Mock data for a PR comment.
    #[derive(Clone, Debug)]
    pub struct MockComment {
        pub author: String,
        pub body: String,
        pub created_at: DateTime<Utc>,
    }

    /// Mock implementation for testing.
    #[derive(Clone, Default)]
    pub struct MockReviewClient {
        /// Reviews on the PR.
        pub reviews: Arc<Mutex<Vec<MockReview>>>,
        /// Comments on the PR.
        pub comments: Arc<Mutex<Vec<MockComment>>>,
        /// Latest commit time.
        pub latest_commit_time: Arc<Mutex<Option<DateTime<Utc>>>>,
        /// Posted comments (for assertions).
        pub posted_comments: Arc<Mutex<Vec<(String, String, u64, Reviewer)>>>,
        /// Error to return (if any).
        pub error: Arc<Mutex<Option<ReviewError>>>,
        /// Number of find_latest_review calls before returning reviews.
        /// If set, the first N calls will return None.
        pub skip_first_n_reviews: Arc<Mutex<usize>>,
        /// Counter for find_latest_review calls.
        pub find_review_call_count: Arc<Mutex<usize>>,
        /// Cutoff time: only return reviews before this time on first call.
        /// After first call, return all reviews.
        pub initial_review_cutoff: Arc<Mutex<Option<DateTime<Utc>>>>,
    }

    impl MockReviewClient {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with_review(self, reviewer: Reviewer, created_at: DateTime<Utc>) -> Self {
            self.reviews.lock().unwrap().push(MockReview {
                reviewer,
                created_at,
            });
            self
        }

        pub fn with_comment(
            self,
            author: impl Into<String>,
            body: impl Into<String>,
            created_at: DateTime<Utc>,
        ) -> Self {
            self.comments.lock().unwrap().push(MockComment {
                author: author.into(),
                body: body.into(),
                created_at,
            });
            self
        }

        pub fn with_latest_commit_time(self, time: DateTime<Utc>) -> Self {
            *self.latest_commit_time.lock().unwrap() = Some(time);
            self
        }

        #[allow(dead_code)]
        pub fn with_error(self, error: ReviewError) -> Self {
            *self.error.lock().unwrap() = Some(error);
            self
        }

        /// Skip the first N calls to find_latest_review (return None).
        /// Useful for simulating "no review initially, then review appears".
        pub fn skip_first_n_review_calls(self, n: usize) -> Self {
            *self.skip_first_n_reviews.lock().unwrap() = n;
            self
        }

        /// Set a cutoff time for initial review check.
        /// On first call, only reviews before this time are returned.
        /// On subsequent calls, all reviews are returned.
        /// Useful for simulating "existing review is old, new review appears during polling".
        pub fn with_initial_review_cutoff(self, cutoff: DateTime<Utc>) -> Self {
            *self.initial_review_cutoff.lock().unwrap() = Some(cutoff);
            self
        }

        /// Add a review dynamically (for use during test execution).
        #[allow(dead_code)]
        pub fn add_review(&self, reviewer: Reviewer, created_at: DateTime<Utc>) {
            self.reviews.lock().unwrap().push(MockReview {
                reviewer,
                created_at,
            });
        }
    }

    #[async_trait::async_trait]
    impl ReviewClient for MockReviewClient {
        async fn find_latest_review(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
            reviewer: Reviewer,
        ) -> Result<Option<DateTime<Utc>>> {
            if let Some(err) = self.error.lock().unwrap().take() {
                return Err(err);
            }

            // Increment call counter
            let mut call_count = self.find_review_call_count.lock().unwrap();
            *call_count += 1;
            let current_call = *call_count;
            drop(call_count);

            // Check if we should skip this call
            let skip_n = *self.skip_first_n_reviews.lock().unwrap();
            if current_call <= skip_n {
                return Ok(None);
            }

            let reviews = self.reviews.lock().unwrap();
            let cutoff = *self.initial_review_cutoff.lock().unwrap();

            let latest = reviews
                .iter()
                .filter(|r| r.reviewer == reviewer)
                .filter(|r| {
                    // On first call, apply cutoff filter if set
                    if current_call == 1 || current_call == skip_n + 1 {
                        cutoff.map_or(true, |c| r.created_at < c)
                    } else {
                        true
                    }
                })
                .map(|r| r.created_at)
                .max();

            Ok(latest)
        }

        async fn check_reviewer_unable_comment(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
            start_time: DateTime<Utc>,
            reviewer: Reviewer,
        ) -> Result<Option<String>> {
            if let Some(err) = self.error.lock().unwrap().take() {
                return Err(err);
            }

            let bot_login = reviewer.bot_login();
            let unable_marker = reviewer.unable_marker();
            let comments = self.comments.lock().unwrap();

            for comment in comments.iter() {
                if comment.author == bot_login
                    && comment.body.contains(unable_marker)
                    && comment.created_at > start_time
                {
                    return Ok(Some(comment.body.clone()));
                }
            }

            Ok(None)
        }

        async fn has_reviewer_activity(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
            reviewer: Reviewer,
        ) -> Result<bool> {
            if let Some(err) = self.error.lock().unwrap().take() {
                return Err(err);
            }

            let bot_login = reviewer.bot_login();
            let comments = self.comments.lock().unwrap();

            Ok(comments.iter().any(|c| c.author == bot_login))
        }

        async fn get_latest_commit_time(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
        ) -> Result<Option<DateTime<Utc>>> {
            if let Some(err) = self.error.lock().unwrap().take() {
                return Err(err);
            }

            Ok(*self.latest_commit_time.lock().unwrap())
        }

        async fn post_review_comment(
            &self,
            owner: &str,
            repo: &str,
            pr_number: u64,
            reviewer: Reviewer,
        ) -> Result<()> {
            if let Some(err) = self.error.lock().unwrap().take() {
                return Err(err);
            }

            self.posted_comments.lock().unwrap().push((
                owner.to_string(),
                repo.to_string(),
                pr_number,
                reviewer,
            ));

            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[tokio::test]
        async fn mock_client_returns_no_review_when_empty() {
            let client = MockReviewClient::new();
            let result = client
                .find_latest_review("owner", "repo", 1, Reviewer::Gemini)
                .await
                .unwrap();
            assert!(result.is_none());
        }

        #[tokio::test]
        async fn mock_client_returns_latest_review() {
            let t1 = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                .unwrap()
                .to_utc();
            let t2 = DateTime::parse_from_rfc3339("2024-01-02T00:00:00Z")
                .unwrap()
                .to_utc();

            let client = MockReviewClient::new()
                .with_review(Reviewer::Gemini, t1)
                .with_review(Reviewer::Gemini, t2);

            let result = client
                .find_latest_review("owner", "repo", 1, Reviewer::Gemini)
                .await
                .unwrap();

            assert_eq!(result, Some(t2));
        }

        #[tokio::test]
        async fn mock_client_detects_unable_comment() {
            let start_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                .unwrap()
                .to_utc();
            let comment_time = DateTime::parse_from_rfc3339("2024-01-02T00:00:00Z")
                .unwrap()
                .to_utc();

            let client = MockReviewClient::new().with_comment(
                "gemini-code-assist",
                "Gemini is unable to review this PR",
                comment_time,
            );

            let result = client
                .check_reviewer_unable_comment("owner", "repo", 1, start_time, Reviewer::Gemini)
                .await
                .unwrap();

            assert!(result.is_some());
            assert!(result.unwrap().contains("Gemini is unable to"));
        }

        #[tokio::test]
        async fn mock_client_ignores_unable_comment_before_start_time() {
            let comment_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                .unwrap()
                .to_utc();
            let start_time = DateTime::parse_from_rfc3339("2024-01-02T00:00:00Z")
                .unwrap()
                .to_utc();

            let client = MockReviewClient::new().with_comment(
                "gemini-code-assist",
                "Gemini is unable to review this PR",
                comment_time,
            );

            let result = client
                .check_reviewer_unable_comment("owner", "repo", 1, start_time, Reviewer::Gemini)
                .await
                .unwrap();

            assert!(result.is_none());
        }

        #[tokio::test]
        async fn mock_client_tracks_posted_comments() {
            let client = MockReviewClient::new();
            client
                .post_review_comment("owner", "repo", 42, Reviewer::Gemini)
                .await
                .unwrap();

            let posted = client.posted_comments.lock().unwrap();
            assert_eq!(posted.len(), 1);
            assert_eq!(
                posted[0],
                (
                    "owner".to_string(),
                    "repo".to_string(),
                    42,
                    Reviewer::Gemini
                )
            );
        }

        #[tokio::test]
        async fn mock_client_returns_latest_commit_time() {
            let commit_time = DateTime::parse_from_rfc3339("2024-01-01T12:00:00Z")
                .unwrap()
                .to_utc();

            let client = MockReviewClient::new().with_latest_commit_time(commit_time);

            let result = client
                .get_latest_commit_time("owner", "repo", 1)
                .await
                .unwrap();

            assert_eq!(result, Some(commit_time));
        }

        #[tokio::test]
        async fn mock_client_has_reviewer_activity() {
            let client = MockReviewClient::new().with_comment(
                "gemini-code-assist",
                "Starting review...",
                Utc::now(),
            );

            let has_activity = client
                .has_reviewer_activity("owner", "repo", 1, Reviewer::Gemini)
                .await
                .unwrap();

            assert!(has_activity);
        }

        #[tokio::test]
        async fn mock_client_no_reviewer_activity() {
            let client =
                MockReviewClient::new().with_comment("other-user", "Some comment", Utc::now());

            let has_activity = client
                .has_reviewer_activity("owner", "repo", 1, Reviewer::Gemini)
                .await
                .unwrap();

            assert!(!has_activity);
        }
    }
}
