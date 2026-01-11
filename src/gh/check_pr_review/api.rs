use super::models::{PrData, Review, ReviewThread};
use super::{CheckPrReviewError, Result};
use indoc::indoc;
use serde::Deserialize;
use std::process::Command;

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
struct GraphQLResponse {
    data: Option<GraphQLData>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQLError {
    message: String,
}

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

pub fn fetch_pr_data(
    owner: &str,
    repo: &str,
    pr_number: u64,
    include_resolved: bool,
) -> Result<PrData> {
    let mut threads: Vec<ReviewThread> = Vec::new();
    let mut reviews: Vec<Review> = Vec::new();
    let mut pagination = PaginationState::default();

    while pagination.has_more() {
        let pr = execute_graphql(owner, repo, pr_number, &pagination)?
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

fn execute_graphql(
    owner: &str,
    repo: &str,
    pr_number: u64,
    pagination: &PaginationState,
) -> Result<GraphQLData> {
    let mut args = vec![
        "api", "graphql", "-f", "query=", "-f", "owner=", "-f", "repo=", "-F", "pr=",
    ];

    // Build the actual argument values separately to avoid shell injection
    let query_arg = format!("query={GRAPHQL_QUERY}");
    let owner_arg = format!("owner={owner}");
    let repo_arg = format!("repo={repo}");
    let pr_arg = format!("pr={pr_number}");

    args[3] = &query_arg;
    args[5] = &owner_arg;
    args[7] = &repo_arg;
    args[9] = &pr_arg;

    let thread_cursor_arg;
    if let Some(cursor) = &pagination.thread_cursor {
        thread_cursor_arg = format!("threadCursor={cursor}");
        args.push("-f");
        args.push(&thread_cursor_arg);
    }

    let review_cursor_arg;
    if let Some(cursor) = &pagination.review_cursor {
        review_cursor_arg = format!("reviewCursor={cursor}");
        args.push("-f");
        args.push(&review_cursor_arg);
    }

    let output = Command::new("gh")
        .args(&args)
        .output()
        .map_err(CheckPrReviewError::IoError)?;

    if !output.status.success() {
        return Err(CheckPrReviewError::GraphQLError(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    let response: GraphQLResponse = serde_json::from_slice(&output.stdout)?;

    if let Some(errors) = response.errors {
        let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        return Err(CheckPrReviewError::GraphQLError(messages.join(", ")));
    }

    response
        .data
        .ok_or_else(|| CheckPrReviewError::GraphQLError("No data in response".to_string()))
}
