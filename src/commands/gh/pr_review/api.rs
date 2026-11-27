use super::Result;
use super::error::CheckPrReviewError;
use super::models::{PrData, Review, ReviewThread};
use crate::infra::github::GitHubClient;
use crate::infra::github::PrState;
use crate::shared::human_in_the_loop::ReviewFinishReason;
use indoc::indoc;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;

// Note: comments(first: 100) doesn't paginate, so threads with 100+ comments
// will be truncated. This is an acceptable limitation as such threads are rare.
const GRAPHQL_QUERY: &str = indoc! {"
    query($owner: String!, $repo: String!, $pr: Int!, $threadCursor: String, $reviewCursor: String) {
      repository(owner: $owner, name: $repo) {
        pullRequest(number: $pr) {
          reviews(first: 100, after: $reviewCursor) {
            pageInfo {
              hasNextPage
              endCursor
            }
            nodes {
              databaseId
              author { login }
              body
              state
              createdAt
              commit { oid }
            }
          }
          reviewThreads(first: 100, after: $threadCursor) {
            pageInfo {
              hasNextPage
              endCursor
            }
            nodes {
              id
              isResolved
              diffSide
              startDiffSide
              comments(first: 100) {
                nodes {
                  databaseId
                  author { login }
                  body
                  createdAt
                  path
                  line
                  startLine
                  originalLine
                  originalStartLine
                  diffHunk
                  replyTo { databaseId }
                  pullRequestReview { databaseId }
                }
              }
            }
          }
        }
      }
    }
"};

const REPLY_REVIEW_STATUS_QUERY: &str = indoc! {"
    query($owner: String!, $repo: String!, $pr: Int!, $threadCursor: String) {
      repository(owner: $owner, name: $repo) {
        pullRequest(number: $pr) {
          state
          mergedAt
          reviewThreads(first: 100, after: $threadCursor) {
            pageInfo {
              hasNextPage
              endCursor
            }
            nodes {
              id
              isResolved
            }
          }
        }
      }
    }
"};

#[derive(Debug, Deserialize)]
struct GraphQLData {
    repository: Option<Repository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    pull_request: Option<PullRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequest {
    reviews: PagedReviews,
    review_threads: PagedThreads,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PagedReviews {
    page_info: PageInfo,
    nodes: Vec<Review>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PagedThreads {
    page_info: PageInfo,
    nodes: Vec<ReviewThread>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReplyReviewStatus {
    pub state: PrState,
    pub resolved_thread_ids: HashSet<String>,
}

impl ReplyReviewStatus {
    pub(super) fn completion_reason(
        &self,
        drafted_thread_ids: &HashSet<String>,
    ) -> Option<ReviewFinishReason> {
        match &self.state {
            PrState::Merged => Some(ReviewFinishReason::PullRequestMerged),
            PrState::Closed => Some(ReviewFinishReason::PullRequestClosed),
            PrState::Open
                if !drafted_thread_ids.is_empty()
                    && drafted_thread_ids
                        .iter()
                        .all(|thread_id| self.resolved_thread_ids.contains(thread_id)) =>
            {
                Some(ReviewFinishReason::ReplyThreadsResolved)
            }
            PrState::Open => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ReplyReviewStatusData {
    repository: Option<ReplyReviewRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplyReviewRepository {
    pull_request: Option<ReplyReviewPullRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplyReviewPullRequest {
    state: PullRequestState,
    merged_at: Option<String>,
    review_threads: PagedReplyReviewThreads,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum PullRequestState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PagedReplyReviewThreads {
    page_info: PageInfo,
    nodes: Vec<ReplyReviewThread>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplyReviewThread {
    id: String,
    is_resolved: bool,
}

#[derive(Default)]
struct PaginationState {
    thread_cursor: Option<String>,
    review_cursor: Option<String>,
    thread_done: bool,
    review_done: bool,
}

impl PaginationState {
    fn has_more(&self) -> bool {
        !self.thread_done || !self.review_done
    }

    fn update_threads(&mut self, page_info: &PageInfo) {
        if page_info.has_next_page {
            self.thread_cursor = page_info.end_cursor.clone();
        } else {
            self.thread_done = true;
        }
    }

    fn update_reviews(&mut self, page_info: &PageInfo) {
        if page_info.has_next_page {
            self.review_cursor = page_info.end_cursor.clone();
        } else {
            self.review_done = true;
        }
    }
}

pub async fn fetch_pr_data(
    owner: &str,
    repo: &str,
    pr_number: u64,
    include_resolved: bool,
) -> Result<PrData> {
    let client = GitHubClient::get()?;
    let mut threads: Vec<ReviewThread> = Vec::new();
    let mut reviews: Vec<Review> = Vec::new();
    let mut pagination = PaginationState::default();

    while pagination.has_more() {
        let pr = execute_graphql(client, owner, repo, pr_number, &pagination)
            .await?
            .repository
            .and_then(|r| r.pull_request)
            .ok_or_else(|| {
                CheckPrReviewError::GraphQLError("Pull request not found".to_string())
            })?;

        if !pagination.thread_done {
            threads.extend(pr.review_threads.nodes);
            pagination.update_threads(&pr.review_threads.page_info);
        }

        if !pagination.review_done {
            reviews.extend(pr.reviews.nodes);
            pagination.update_reviews(&pr.reviews.page_info);
        }
    }

    if !include_resolved {
        threads.retain(|t| !t.is_resolved);
    }

    Ok(PrData { reviews, threads })
}

pub(super) async fn fetch_reply_review_status(
    client: &GitHubClient,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> Result<ReplyReviewStatus> {
    let mut cursor = None;
    let mut pull_request_state = None;
    let mut resolved_thread_ids = std::collections::HashSet::new();

    loop {
        let response = client
            .graphql::<ReplyReviewStatusData>(
                REPLY_REVIEW_STATUS_QUERY,
                json!({
                    "owner": owner,
                    "repo": repo,
                    "pr": pr_number,
                    "threadCursor": cursor,
                }),
            )
            .await?;
        let pull_request = response
            .repository
            .and_then(|repository| repository.pull_request)
            .ok_or_else(|| {
                CheckPrReviewError::GraphQLError("Pull request not found".to_string())
            })?;

        let state = if pull_request.merged_at.is_some() {
            PrState::Merged
        } else {
            match pull_request.state {
                PullRequestState::Open => PrState::Open,
                PullRequestState::Closed => PrState::Closed,
                PullRequestState::Merged => PrState::Merged,
            }
        };
        if pull_request_state
            .as_ref()
            .is_some_and(|previous| previous != &state)
        {
            return Err(CheckPrReviewError::GraphQLError(
                "Pull request state changed while fetching review threads".to_string(),
            )
            .into());
        }
        pull_request_state = Some(state);

        let page = pull_request.review_threads;
        resolved_thread_ids.extend(
            page.nodes
                .into_iter()
                .filter(|thread| thread.is_resolved)
                .map(|thread| thread.id),
        );
        if !page.page_info.has_next_page {
            break;
        }
        cursor = page.page_info.end_cursor;
        if cursor.is_none() {
            return Err(CheckPrReviewError::GraphQLError(
                "Pull request review thread pagination returned no cursor".to_string(),
            )
            .into());
        }
    }

    Ok(ReplyReviewStatus {
        state: pull_request_state.ok_or_else(|| {
            CheckPrReviewError::GraphQLError("Pull request status was not returned".to_string())
        })?,
        resolved_thread_ids,
    })
}

async fn execute_graphql(
    client: &GitHubClient,
    owner: &str,
    repo: &str,
    pr_number: u64,
    pagination: &PaginationState,
) -> Result<GraphQLData> {
    let variables = json!({
        "owner": owner,
        "repo": repo,
        "pr": pr_number,
        "threadCursor": pagination.thread_cursor,
        "reviewCursor": pagination.review_cursor,
    });

    client.graphql(GRAPHQL_QUERY, variables).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::github::GitHubMockServer;
    use serde_json::json;

    #[tokio::test]
    async fn reply_review_status_returns_pr_state_and_resolved_thread_ids() {
        let mock = GitHubMockServer::start().await;
        mock.graphql_reply_review_status(
            "OPEN",
            None,
            None,
            false,
            vec![
                json!({"id": "PRRT_fake_resolved", "isResolved": true}),
                json!({"id": "PRRT_fake_unresolved", "isResolved": false}),
            ],
        )
        .await;

        let actual = fetch_reply_review_status(&mock.client(), "sample-org", "sample-repo", 17)
            .await
            .expect("reply review status should be fetched");

        assert_eq!(
            actual,
            ReplyReviewStatus {
                state: PrState::Open,
                resolved_thread_ids: HashSet::from(["PRRT_fake_resolved".to_string()]),
            },
        );
    }

    #[tokio::test]
    async fn reply_review_status_fetches_later_thread_pages() {
        let mock = GitHubMockServer::start().await;
        mock.graphql_reply_review_status(
            "OPEN",
            None,
            Some("next-page"),
            true,
            vec![json!({"id": "PRRT_fake_first", "isResolved": false})],
        )
        .await;
        mock.graphql_reply_review_status_page(
            Some("next-page"),
            "OPEN",
            None,
            None,
            false,
            vec![json!({"id": "PRRT_fake_second", "isResolved": true})],
        )
        .await;

        let actual = fetch_reply_review_status(&mock.client(), "sample-org", "sample-repo", 17)
            .await
            .expect("all review thread pages should be fetched");

        assert_eq!(
            actual,
            ReplyReviewStatus {
                state: PrState::Open,
                resolved_thread_ids: HashSet::from(["PRRT_fake_second".to_string()]),
            },
        );
    }

    #[rstest::rstest]
    #[case::open("OPEN", None, PrState::Open)]
    #[case::closed("CLOSED", None, PrState::Closed)]
    #[case::merged("MERGED", Some("2026-01-01T00:00:00Z"), PrState::Merged)]
    #[tokio::test]
    async fn reply_review_status_deserializes_pull_request_states(
        #[case] state: &str,
        #[case] merged_at: Option<&str>,
        #[case] expected_state: PrState,
    ) {
        let mock = GitHubMockServer::start().await;
        mock.graphql_reply_review_status(state, merged_at, None, false, Vec::new())
            .await;

        let actual = fetch_reply_review_status(&mock.client(), "sample-org", "sample-repo", 17)
            .await
            .expect("pull request state should be deserialized");

        assert_eq!(
            actual,
            ReplyReviewStatus {
                state: expected_state,
                resolved_thread_ids: HashSet::new(),
            },
        );
    }
}
