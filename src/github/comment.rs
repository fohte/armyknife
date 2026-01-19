//! Comment operations.

use indoc::indoc;
use serde::Deserialize;

use super::client::OctocrabClient;
use super::error::Result;

/// Trait for comment operations.
#[allow(dead_code)]
#[async_trait::async_trait]
pub trait CommentClient: Send + Sync {
    /// Get comments for an issue using GraphQL.
    /// Returns both node ID (for GraphQL) and database ID (for REST API).
    async fn get_comments(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<Vec<crate::gh::issue_agent::models::Comment>>;

    /// Update a comment using REST API.
    async fn update_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        body: &str,
    ) -> Result<()>;

    /// Create a new comment on an issue.
    async fn create_comment(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        body: &str,
    ) -> Result<crate::gh::issue_agent::models::Comment>;

    /// Delete a comment from an issue.
    async fn delete_comment(&self, owner: &str, repo: &str, comment_id: u64) -> Result<()>;
}

/// GraphQL response wrapper containing the data field.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct GraphQLCommentsResponse {
    data: GraphQLCommentsData,
}

/// GraphQL data field for fetching comments.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct GraphQLCommentsData {
    repository: RepositoryData,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct RepositoryData {
    issue: IssueData,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct IssueData {
    comments: CommentsConnection,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct CommentsConnection {
    nodes: Vec<GraphQLComment>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQLComment {
    id: String,
    database_id: i64,
    author: Option<GraphQLAuthor>,
    created_at: chrono::DateTime<chrono::Utc>,
    body: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct GraphQLAuthor {
    login: String,
}

#[allow(dead_code)]
const GET_COMMENTS_QUERY: &str = indoc! {"
    query($owner: String!, $repo: String!, $number: Int!) {
        repository(owner: $owner, name: $repo) {
            issue(number: $number) {
                comments(first: 100) {
                    nodes {
                        id
                        databaseId
                        author { login }
                        createdAt
                        body
                    }
                }
            }
        }
    }
"};

#[async_trait::async_trait]
impl CommentClient for OctocrabClient {
    async fn get_comments(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<Vec<crate::gh::issue_agent::models::Comment>> {
        let variables = serde_json::json!({
            "owner": owner,
            "repo": repo,
            "number": issue_number as i64,
        });

        let response: GraphQLCommentsResponse = self.graphql(GET_COMMENTS_QUERY, variables).await?;

        Ok(response
            .data
            .repository
            .issue
            .comments
            .nodes
            .into_iter()
            .map(|c| crate::gh::issue_agent::models::Comment {
                id: c.id,
                database_id: c.database_id,
                author: c
                    .author
                    .map(|a| crate::gh::issue_agent::models::Author { login: a.login }),
                created_at: c.created_at,
                body: c.body,
            })
            .collect())
    }

    async fn update_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        body: &str,
    ) -> Result<()> {
        // Use REST API: PATCH /repos/{owner}/{repo}/issues/comments/{comment_id}
        let route = format!("/repos/{owner}/{repo}/issues/comments/{comment_id}");
        let _response: serde_json::Value = self
            .client
            .patch(route, Some(&serde_json::json!({ "body": body })))
            .await?;
        Ok(())
    }

    async fn create_comment(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        body: &str,
    ) -> Result<crate::gh::issue_agent::models::Comment> {
        let comment = self
            .client
            .issues(owner, repo)
            .create_comment(issue_number, body)
            .await?;

        Ok(crate::gh::issue_agent::models::Comment {
            id: comment.node_id,
            database_id: comment.id.0 as i64,
            author: Some(crate::gh::issue_agent::models::Author {
                login: comment.user.login,
            }),
            created_at: comment.created_at,
            body: comment.body.unwrap_or_default(),
        })
    }

    async fn delete_comment(&self, owner: &str, repo: &str, comment_id: u64) -> Result<()> {
        // Use REST API: DELETE /repos/{owner}/{repo}/issues/comments/{comment_id}
        let route = format!("/repos/{owner}/{repo}/issues/comments/{comment_id}");
        self.client
            .delete::<(), String, ()>(route, Option::<&()>::None)
            .await?;
        Ok(())
    }
}
