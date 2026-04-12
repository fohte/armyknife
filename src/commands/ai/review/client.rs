//! Production implementation of DetectionClient using GitHub API.

use anyhow::Context;
use chrono::{DateTime, Utc};
use indoc::indoc;
use serde::Deserialize;
use serde_json::json;

use super::detector::DetectionClient;
use super::error::{Result, ReviewError};
use crate::commands::gh::pr_review::fetch_pr_data;
use crate::infra::github::GitHubClient;

/// Production implementation using GitHub API via GitHubClient.
pub struct OctocrabDetectionClient;

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

// GraphQL query to check reactions on PR body
const PR_REACTIONS_QUERY: &str = indoc! {"
    query($owner: String!, $repo: String!, $pr: Int!) {
        repository(owner: $owner, name: $repo) {
            pullRequest(number: $pr) {
                reactions(first: 100) {
                    nodes {
                        content
                    }
                }
            }
        }
    }
"};

// GraphQL query to check check runs on the PR's head commit
const CHECK_RUNS_QUERY: &str = indoc! {"
    query($owner: String!, $repo: String!, $pr: Int!) {
        repository(owner: $owner, name: $repo) {
            pullRequest(number: $pr) {
                commits(last: 1) {
                    nodes {
                        commit {
                            checkSuites(first: 10) {
                                nodes {
                                    checkRuns(first: 50) {
                                        nodes {
                                            name
                                            status
                                            completedAt
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
"};

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

// Reaction query types
#[derive(Debug, Deserialize)]
struct PrReactionsData {
    repository: Option<PrReactionsRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrReactionsRepository {
    pull_request: Option<PrReactionsPullRequest>,
}

#[derive(Debug, Deserialize)]
struct PrReactionsPullRequest {
    reactions: PrReactions,
}

#[derive(Debug, Deserialize)]
struct PrReactions {
    nodes: Vec<PrReaction>,
}

#[derive(Debug, Deserialize)]
struct PrReaction {
    content: String,
}

// Check runs query types
#[derive(Debug, Deserialize)]
struct CheckRunsData {
    repository: Option<CheckRunsRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckRunsRepository {
    pull_request: Option<CheckRunsPullRequest>,
}

#[derive(Debug, Deserialize)]
struct CheckRunsPullRequest {
    commits: CheckRunsCommits,
}

#[derive(Debug, Deserialize)]
struct CheckRunsCommits {
    nodes: Vec<CheckRunsCommitNode>,
}

#[derive(Debug, Deserialize)]
struct CheckRunsCommitNode {
    commit: CheckRunsCommit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckRunsCommit {
    check_suites: CheckSuites,
}

#[derive(Debug, Deserialize)]
struct CheckSuites {
    nodes: Vec<CheckSuite>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckSuite {
    check_runs: CheckRuns,
}

#[derive(Debug, Deserialize)]
struct CheckRuns {
    nodes: Vec<CheckRun>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckRun {
    name: String,
    status: String,
    completed_at: Option<String>,
}

impl OctocrabDetectionClient {
    async fn fetch_pr_info(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<PrInfoPullRequest> {
        let client = GitHubClient::get()?;
        let variables = json!({
            "owner": owner,
            "repo": repo,
            "pr": pr_number,
        });

        let response: PrInfoData = client.graphql(PR_INFO_QUERY, variables).await?;

        response
            .repository
            .and_then(|r| r.pull_request)
            .ok_or_else(|| ReviewError::RepoInfoError("Pull request not found".to_string()).into())
    }

    fn find_check_runs<'a>(
        &self,
        check_runs_data: &'a CheckRunsData,
        check_name: &str,
    ) -> Vec<&'a CheckRun> {
        check_runs_data
            .repository
            .iter()
            .flat_map(|r| r.pull_request.iter())
            .flat_map(|pr| &pr.commits.nodes)
            .flat_map(|cn| &cn.commit.check_suites.nodes)
            .flat_map(|cs| &cs.check_runs.nodes)
            .filter(|cr| cr.name == check_name)
            .collect()
    }
}

impl DetectionClient for OctocrabDetectionClient {
    async fn find_latest_review(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        bot_login: &str,
    ) -> Result<Option<DateTime<Utc>>> {
        let pr_data = fetch_pr_data(owner, repo, pr_number, true)
            .await
            .context("Failed to fetch PR data")?;

        pr_data
            .reviews
            .iter()
            .filter(|review| review.author.as_ref().is_some_and(|a| a.login == bot_login))
            .try_fold(None, |acc, review| {
                review
                    .created_at
                    .parse::<DateTime<Utc>>()
                    .map(|created_at| {
                        Some(acc.map_or(created_at, |prev: DateTime<Utc>| prev.max(created_at)))
                    })
                    .map_err(|_| ReviewError::TimestampParseError(review.created_at.clone()).into())
            })
    }

    async fn find_unable_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        bot_login: &str,
        unable_marker: &str,
        after: DateTime<Utc>,
    ) -> Result<Option<String>> {
        let pr_info = self.fetch_pr_info(owner, repo, pr_number).await?;

        Ok(pr_info
            .comments
            .nodes
            .iter()
            .find(|comment| {
                comment
                    .author
                    .as_ref()
                    .is_some_and(|a| a.login == bot_login)
                    && comment.body.contains(unable_marker)
                    && comment
                        .created_at
                        .parse::<DateTime<Utc>>()
                        .is_ok_and(|created_at| created_at > after)
            })
            .map(|comment| comment.body.clone()))
    }

    async fn get_latest_commit_time(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<Option<DateTime<Utc>>> {
        let pr_info = self.fetch_pr_info(owner, repo, pr_number).await?;

        pr_info
            .commits
            .nodes
            .first()
            .map(|commit_node| {
                commit_node
                    .commit
                    .committed_date
                    .parse::<DateTime<Utc>>()
                    .map_err(|_| {
                        ReviewError::TimestampParseError(commit_node.commit.committed_date.clone())
                            .into()
                    })
            })
            .transpose()
    }

    async fn post_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        body: &str,
    ) -> Result<()> {
        let client = GitHubClient::get()?;

        client
            .create_comment(owner, repo, pr_number, body)
            .await
            .context("Failed to post comment")?;

        Ok(())
    }

    async fn has_body_reaction(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        emoji: &str,
    ) -> Result<bool> {
        let client = GitHubClient::get()?;
        let variables = json!({
            "owner": owner,
            "repo": repo,
            "pr": pr_number,
        });

        let response: PrReactionsData = client.graphql(PR_REACTIONS_QUERY, variables).await?;

        // GitHub GraphQL uses UPPER_SNAKE_CASE for reaction content (e.g., "EYES")
        let graphql_emoji = emoji.to_uppercase();

        Ok(response
            .repository
            .and_then(|r| r.pull_request)
            .map(|pr| {
                pr.reactions
                    .nodes
                    .iter()
                    .any(|r| r.content == graphql_emoji)
            })
            .unwrap_or(false))
    }

    async fn has_check_run(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        check_name: &str,
    ) -> Result<bool> {
        let client = GitHubClient::get()?;
        let variables = json!({
            "owner": owner,
            "repo": repo,
            "pr": pr_number,
        });

        let response: CheckRunsData = client.graphql(CHECK_RUNS_QUERY, variables).await?;

        Ok(!self.find_check_runs(&response, check_name).is_empty())
    }

    async fn check_run_completed_at(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        check_name: &str,
    ) -> Result<Option<DateTime<Utc>>> {
        let client = GitHubClient::get()?;
        let variables = json!({
            "owner": owner,
            "repo": repo,
            "pr": pr_number,
        });

        let response: CheckRunsData = client.graphql(CHECK_RUNS_QUERY, variables).await?;

        let check_runs = self.find_check_runs(&response, check_name);

        for cr in check_runs {
            if cr.status == "COMPLETED"
                && let Some(ref completed_at) = cr.completed_at
            {
                let t = completed_at
                    .parse::<DateTime<Utc>>()
                    .map_err(|_| ReviewError::TimestampParseError(completed_at.clone()))?;
                return Ok(Some(t));
            }
        }

        Ok(None)
    }
}

/// Get the default production client.
pub fn get_client() -> OctocrabDetectionClient {
    OctocrabDetectionClient
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::commands::ai::review::detector::ReviewDetector;
    use crate::commands::ai::review::reviewer::Reviewer;

    /// Mock data for a PR review.
    #[derive(Clone, Debug)]
    pub struct MockReview {
        pub bot_login: String,
        pub created_at: DateTime<Utc>,
    }

    /// Mock data for a PR comment.
    #[derive(Clone, Debug)]
    pub struct MockComment {
        pub author: String,
        pub body: String,
        pub created_at: DateTime<Utc>,
    }

    /// Posted comment data for test assertions.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct PostedComment {
        pub owner: String,
        pub repo: String,
        pub pr_number: u64,
        pub body: String,
    }

    /// Mock data for a check run.
    #[derive(Clone, Debug)]
    pub struct MockCheckRun {
        pub name: String,
        pub completed_at: Option<DateTime<Utc>>,
    }

    /// Mock data for a reaction on the PR body.
    #[derive(Clone, Debug)]
    pub struct MockReaction {
        pub emoji: String,
    }

    /// Mock implementation for testing.
    #[derive(Clone, Default)]
    pub struct MockDetectionClient {
        /// Reviews on the PR.
        pub reviews: Arc<Mutex<Vec<MockReview>>>,
        /// Comments on the PR.
        pub comments: Arc<Mutex<Vec<MockComment>>>,
        /// Latest commit time.
        pub latest_commit_time: Arc<Mutex<Option<DateTime<Utc>>>>,
        /// Posted comments (for assertions).
        pub posted_comments: Arc<Mutex<Vec<PostedComment>>>,
        /// Check runs on the PR's head commit.
        pub check_runs: Arc<Mutex<Vec<MockCheckRun>>>,
        /// Reactions on the PR body.
        pub reactions: Arc<Mutex<Vec<MockReaction>>>,
        /// Number of find_latest_review calls before returning reviews.
        /// If set, the first N calls will return None.
        pub skip_first_n_reviews: Arc<Mutex<usize>>,
        /// Counter for find_latest_review calls.
        pub find_review_call_count: Arc<Mutex<usize>>,
        /// Cutoff time: only return reviews before this time on initial calls.
        pub initial_review_cutoff: Arc<Mutex<Option<DateTime<Utc>>>>,
        /// Number of calls to apply cutoff to (default: 1).
        pub initial_review_cutoff_calls: Arc<Mutex<usize>>,
    }

    impl MockDetectionClient {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with_review(self, reviewer: Reviewer, created_at: DateTime<Utc>) -> Self {
            self.reviews.lock().unwrap().push(MockReview {
                bot_login: reviewer.detector().bot_login().to_string(),
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

        pub fn with_check_run(
            self,
            name: impl Into<String>,
            completed_at: Option<DateTime<Utc>>,
        ) -> Self {
            self.check_runs.lock().unwrap().push(MockCheckRun {
                name: name.into(),
                completed_at,
            });
            self
        }

        pub fn with_reaction(self, emoji: impl Into<String>) -> Self {
            self.reactions.lock().unwrap().push(MockReaction {
                emoji: emoji.into(),
            });
            self
        }

        /// Skip the first N calls to find_latest_review (return None).
        /// Useful for simulating "no review initially, then review appears".
        pub fn skip_first_n_review_calls(self, n: usize) -> Self {
            *self.skip_first_n_reviews.lock().unwrap() = n;
            self
        }

        /// Set a cutoff time for initial review check.
        /// On the first `num_calls` calls, only reviews before `cutoff` are returned.
        /// On subsequent calls, all reviews are returned.
        /// Useful for simulating "existing review is old, new review appears during polling".
        pub fn with_initial_review_cutoff(self, cutoff: DateTime<Utc>, num_calls: usize) -> Self {
            *self.initial_review_cutoff.lock().unwrap() = Some(cutoff);
            *self.initial_review_cutoff_calls.lock().unwrap() = num_calls;
            self
        }
    }

    impl DetectionClient for MockDetectionClient {
        async fn find_latest_review(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
            bot_login: &str,
        ) -> Result<Option<DateTime<Utc>>> {
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
            let cutoff_calls = *self.initial_review_cutoff_calls.lock().unwrap();

            // Determine effective call number (after skipping)
            let effective_call = current_call.saturating_sub(skip_n);

            let latest = reviews
                .iter()
                .filter(|r| r.bot_login == bot_login)
                .filter(|r| {
                    // Apply cutoff filter on initial calls (up to cutoff_calls)
                    if effective_call > 0 && effective_call <= cutoff_calls {
                        cutoff.is_none_or(|c| r.created_at < c)
                    } else {
                        true
                    }
                })
                .map(|r| r.created_at)
                .max();

            Ok(latest)
        }

        async fn find_unable_comment(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
            bot_login: &str,
            unable_marker: &str,
            after: DateTime<Utc>,
        ) -> Result<Option<String>> {
            let comments = self.comments.lock().unwrap();

            for comment in comments.iter() {
                if comment.author == bot_login
                    && comment.body.contains(unable_marker)
                    && comment.created_at > after
                {
                    return Ok(Some(comment.body.clone()));
                }
            }

            Ok(None)
        }

        async fn get_latest_commit_time(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
        ) -> Result<Option<DateTime<Utc>>> {
            Ok(*self.latest_commit_time.lock().unwrap())
        }

        async fn post_comment(
            &self,
            owner: &str,
            repo: &str,
            pr_number: u64,
            body: &str,
        ) -> Result<()> {
            self.posted_comments.lock().unwrap().push(PostedComment {
                owner: owner.to_string(),
                repo: repo.to_string(),
                pr_number,
                body: body.to_string(),
            });

            Ok(())
        }

        async fn has_body_reaction(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
            emoji: &str,
        ) -> Result<bool> {
            let reactions = self.reactions.lock().unwrap();
            Ok(reactions.iter().any(|r| r.emoji == emoji))
        }

        async fn has_check_run(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
            check_name: &str,
        ) -> Result<bool> {
            let check_runs = self.check_runs.lock().unwrap();
            Ok(check_runs.iter().any(|cr| cr.name == check_name))
        }

        async fn check_run_completed_at(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
            check_name: &str,
        ) -> Result<Option<DateTime<Utc>>> {
            let check_runs = self.check_runs.lock().unwrap();
            Ok(check_runs
                .iter()
                .find(|cr| cr.name == check_name && cr.completed_at.is_some())
                .and_then(|cr| cr.completed_at))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use rstest::rstest;

        #[tokio::test]
        async fn find_latest_review_returns_none_when_empty() {
            let client = MockDetectionClient::new();
            let result = client
                .find_latest_review("owner", "repo", 1, "gemini-code-assist")
                .await
                .unwrap();
            assert!(result.is_none());
        }

        #[tokio::test]
        async fn find_latest_review_returns_latest() {
            let t1 = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                .unwrap()
                .to_utc();
            let t2 = DateTime::parse_from_rfc3339("2024-01-02T00:00:00Z")
                .unwrap()
                .to_utc();

            let client = MockDetectionClient::new()
                .with_review(Reviewer::Gemini, t1)
                .with_review(Reviewer::Gemini, t2);

            let result = client
                .find_latest_review("owner", "repo", 1, "gemini-code-assist")
                .await
                .unwrap();

            assert_eq!(result, Some(t2));
        }

        #[rstest]
        #[case::after_start_time("2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z", true)]
        #[case::before_start_time("2024-01-02T00:00:00Z", "2024-01-01T00:00:00Z", false)]
        #[tokio::test]
        async fn check_unable_comment_respects_start_time(
            #[case] start_time_str: &str,
            #[case] comment_time_str: &str,
            #[case] expected_some: bool,
        ) {
            let start_time = DateTime::parse_from_rfc3339(start_time_str)
                .unwrap()
                .to_utc();
            let comment_time = DateTime::parse_from_rfc3339(comment_time_str)
                .unwrap()
                .to_utc();

            let client = MockDetectionClient::new().with_comment(
                "gemini-code-assist",
                "Gemini is unable to review this PR",
                comment_time,
            );

            let result = client
                .find_unable_comment(
                    "owner",
                    "repo",
                    1,
                    "gemini-code-assist",
                    "Gemini is unable to",
                    start_time,
                )
                .await
                .unwrap();

            assert_eq!(result.is_some(), expected_some);
        }

        #[tokio::test]
        async fn post_comment_tracks_calls() {
            let client = MockDetectionClient::new();
            client
                .post_comment("owner", "repo", 42, "/gemini review")
                .await
                .unwrap();

            let posted = client.posted_comments.lock().unwrap();
            assert_eq!(posted.len(), 1);
            assert_eq!(
                posted[0],
                PostedComment {
                    owner: "owner".to_string(),
                    repo: "repo".to_string(),
                    pr_number: 42,
                    body: "/gemini review".to_string(),
                }
            );
        }

        #[tokio::test]
        async fn get_latest_commit_time_returns_configured_time() {
            let commit_time = DateTime::parse_from_rfc3339("2024-01-01T12:00:00Z")
                .unwrap()
                .to_utc();

            let client = MockDetectionClient::new().with_latest_commit_time(commit_time);

            let result = client
                .get_latest_commit_time("owner", "repo", 1)
                .await
                .unwrap();

            assert_eq!(result, Some(commit_time));
        }

        #[rstest]
        #[case::has_reaction("eyes", true)]
        #[case::no_reaction("thumbsup", false)]
        #[tokio::test]
        async fn has_body_reaction_checks_emoji(#[case] query: &str, #[case] expected: bool) {
            let client = MockDetectionClient::new().with_reaction("eyes");

            let result = client
                .has_body_reaction("owner", "repo", 1, query)
                .await
                .unwrap();

            assert_eq!(result, expected);
        }

        #[rstest]
        #[case::exists("devin-review", true)]
        #[case::missing("other-check", false)]
        #[tokio::test]
        async fn has_check_run_checks_name(#[case] query: &str, #[case] expected: bool) {
            let client = MockDetectionClient::new().with_check_run("devin-review", None);

            let result = client
                .has_check_run("owner", "repo", 1, query)
                .await
                .unwrap();

            assert_eq!(result, expected);
        }

        #[tokio::test]
        async fn check_run_completed_at_returns_time_when_completed() {
            let t = DateTime::parse_from_rfc3339("2024-01-01T12:00:00Z")
                .unwrap()
                .to_utc();
            let client = MockDetectionClient::new().with_check_run("devin-review", Some(t));

            let result = client
                .check_run_completed_at("owner", "repo", 1, "devin-review")
                .await
                .unwrap();

            assert_eq!(result, Some(t));
        }

        #[tokio::test]
        async fn check_run_completed_at_returns_none_when_in_progress() {
            let client = MockDetectionClient::new().with_check_run("devin-review", None);

            let result = client
                .check_run_completed_at("owner", "repo", 1, "devin-review")
                .await
                .unwrap();

            assert!(result.is_none());
        }
    }
}
