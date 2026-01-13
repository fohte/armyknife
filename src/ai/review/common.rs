//! Common utilities for review commands.

use super::error::{Result, ReviewError};
use super::reviewer::Reviewer;
use crate::gh::check_pr_review::fetch_pr_data;
use crate::git;
use crate::github::{OctocrabClient, PrClient, PrState};
use chrono::{DateTime, Utc};
use indoc::indoc;
use serde::Deserialize;
use serde_json::json;

/// Get repository owner and name from argument or git remote
pub fn get_repo_owner_and_name(repo_arg: Option<&str>) -> Result<(String, String)> {
    if let Some(repo) = repo_arg {
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() == 2 {
            return Ok((parts[0].to_string(), parts[1].to_string()));
        }
        return Err(ReviewError::RepoInfoError(format!(
            "Invalid repository format: {repo}. Expected owner/repo"
        )));
    }

    let repo = git::open_repo()?;
    let (owner, name) = git::github_owner_and_repo(&repo)?;
    Ok((owner, name))
}

/// Get PR number from argument or by finding PR for current branch
pub async fn get_pr_number(owner: &str, repo: &str, pr_arg: Option<u64>) -> Result<u64> {
    if let Some(pr) = pr_arg {
        return Ok(pr);
    }

    // Get current branch name
    let git_repo = git::open_repo()?;
    let branch = git::current_branch(&git_repo)?;

    // Find PR for this branch
    let client = OctocrabClient::get()?;
    let pr_info = client.get_pr_for_branch(owner, repo, &branch).await?;

    match pr_info {
        Some(info) if info.state == PrState::Open => {
            // Extract PR number from URL (e.g., https://github.com/owner/repo/pull/123)
            let pr_number = info
                .url
                .rsplit('/')
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| {
                    ReviewError::RepoInfoError("Failed to parse PR number from URL".into())
                })?;
            Ok(pr_number)
        }
        _ => Err(ReviewError::NoPrFound),
    }
}

/// Find the latest review timestamp from the specified reviewer
pub async fn find_latest_review(
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
struct PrInfoComment {
    author: Option<PrInfoAuthor>,
    body: String,
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct PrInfoAuthor {
    login: String,
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

/// Fetch PR info (comments and latest commit) using GraphQL
async fn fetch_pr_info(owner: &str, repo: &str, pr_number: u64) -> Result<PrInfoPullRequest> {
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

/// Check if the reviewer posted an "unable to" comment after start_time
pub async fn check_reviewer_unable_comment(
    owner: &str,
    repo: &str,
    pr_number: u64,
    start_time: DateTime<Utc>,
    reviewer: Reviewer,
) -> Result<Option<String>> {
    let bot_login = reviewer.bot_login();
    let unable_marker = reviewer.unable_marker();

    let pr_info = fetch_pr_info(owner, repo, pr_number).await?;

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

/// Check if the reviewer has any activity (comments) on the PR
pub async fn has_reviewer_activity(
    owner: &str,
    repo: &str,
    pr_number: u64,
    reviewer: Reviewer,
) -> Result<bool> {
    let bot_login = reviewer.bot_login();
    let pr_info = fetch_pr_info(owner, repo, pr_number).await?;

    Ok(pr_info
        .comments
        .nodes
        .iter()
        .any(|c| c.author.as_ref().is_some_and(|a| a.login == bot_login)))
}

/// Get the latest commit timestamp on the PR
pub async fn get_latest_commit_time(
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> Result<Option<DateTime<Utc>>> {
    let pr_info = fetch_pr_info(owner, repo, pr_number).await?;

    let Some(commit_node) = pr_info.commits.nodes.first() else {
        return Ok(None);
    };

    commit_node
        .commit
        .committed_date
        .parse::<DateTime<Utc>>()
        .map(Some)
        .map_err(|_| ReviewError::TimestampParseError(commit_node.commit.committed_date.clone()))
}

/// Post a review request comment using octocrab
pub async fn post_review_comment(
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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::valid("owner/repo", "owner", "repo")]
    #[case::with_dashes("my-org/my-repo", "my-org", "my-repo")]
    #[case::with_numbers("user123/project456", "user123", "project456")]
    fn test_get_repo_owner_and_name_with_arg(
        #[case] input: &str,
        #[case] expected_owner: &str,
        #[case] expected_repo: &str,
    ) {
        let (owner, repo) = get_repo_owner_and_name(Some(input)).unwrap();
        assert_eq!(owner, expected_owner);
        assert_eq!(repo, expected_repo);
    }

    #[rstest]
    #[case::no_slash("invalid")]
    #[case::too_many_slashes("a/b/c")]
    #[case::empty("")]
    fn test_get_repo_owner_and_name_invalid(#[case] input: &str) {
        let result = get_repo_owner_and_name(Some(input));
        assert!(result.is_err());
        assert!(matches!(result, Err(ReviewError::RepoInfoError(_))));
    }
}
