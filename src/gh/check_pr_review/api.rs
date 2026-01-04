use super::models::{PrData, Review, ReviewThread};
use super::{CheckPrReviewError, Result};
use serde::Deserialize;
use std::process::Command;

const GRAPHQL_QUERY: &str = r#"
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
"#;

#[derive(Debug, Deserialize)]
struct GraphQlResponse {
    data: Option<GraphQlData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct GraphQlData {
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

pub fn fetch_pr_data(
    owner: &str,
    repo: &str,
    pr_number: u64,
    include_resolved: bool,
) -> Result<PrData> {
    let mut all_threads: Vec<ReviewThread> = Vec::new();
    let mut all_reviews: Vec<Review> = Vec::new();
    let mut thread_cursor: Option<String> = None;
    let mut review_cursor: Option<String> = None;
    let mut thread_has_next = true;
    let mut review_has_next = true;

    while thread_has_next || review_has_next {
        let result = execute_graphql(
            owner,
            repo,
            pr_number,
            thread_cursor.as_deref(),
            review_cursor.as_deref(),
        )?;

        let pr = result
            .repository
            .and_then(|r| r.pull_request)
            .ok_or_else(|| {
                CheckPrReviewError::GraphQlError("Pull request not found".to_string())
            })?;

        if thread_has_next {
            all_threads.extend(pr.review_threads.nodes);
            thread_has_next = pr.review_threads.page_info.has_next_page;
            thread_cursor = pr.review_threads.page_info.end_cursor;
        }

        if review_has_next {
            all_reviews.extend(pr.reviews.nodes);
            review_has_next = pr.reviews.page_info.has_next_page;
            review_cursor = pr.reviews.page_info.end_cursor;
        }
    }

    // Filter threads by resolved status if needed
    if !include_resolved {
        all_threads.retain(|t| !t.is_resolved);
    }

    // Filter reviews with non-empty body
    all_reviews.retain(|r| !r.body.is_empty());

    Ok(PrData {
        reviews: all_reviews,
        threads: all_threads,
    })
}

fn execute_graphql(
    owner: &str,
    repo: &str,
    pr_number: u64,
    thread_cursor: Option<&str>,
    review_cursor: Option<&str>,
) -> Result<GraphQlData> {
    let mut args = vec![
        "api".to_string(),
        "graphql".to_string(),
        "-f".to_string(),
        format!("query={GRAPHQL_QUERY}"),
        "-F".to_string(),
        format!("owner={owner}"),
        "-F".to_string(),
        format!("repo={repo}"),
        "-F".to_string(),
        format!("pr={pr_number}"),
    ];

    if let Some(cursor) = thread_cursor {
        args.push("-f".to_string());
        args.push(format!("threadCursor={cursor}"));
    }

    if let Some(cursor) = review_cursor {
        args.push("-f".to_string());
        args.push(format!("reviewCursor={cursor}"));
    }

    let output = Command::new("gh")
        .args(&args)
        .output()
        .map_err(CheckPrReviewError::IoError)?;

    if !output.status.success() {
        return Err(CheckPrReviewError::GraphQlError(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    let response: GraphQlResponse = serde_json::from_slice(&output.stdout)?;

    if let Some(errors) = response.errors {
        let messages: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
        return Err(CheckPrReviewError::GraphQlError(messages.join(", ")));
    }

    response
        .data
        .ok_or_else(|| CheckPrReviewError::GraphQlError("No data in response".to_string()))
}
