use super::models::{PrData, Review, ReviewThread};
use super::{CheckPrReviewError, Result};
use crate::infra::github::OctocrabClient;
use indoc::indoc;
use serde::Deserialize;
use serde_json::json;

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
            }
          }
          reviewThreads(first: 100, after: $threadCursor) {
            pageInfo {
              hasNextPage
              endCursor
            }
            nodes {
              isResolved
              comments(first: 100) {
                nodes {
                  databaseId
                  author { login }
                  body
                  createdAt
                  path
                  line
                  originalLine
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
    let client = OctocrabClient::get()?;
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

async fn execute_graphql(
    client: &OctocrabClient,
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
