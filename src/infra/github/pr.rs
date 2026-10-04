//! Pull request operations.

use std::collections::HashMap;

use serde::Deserialize;

use super::client::GitHubClient;
use super::error::{GitHubError, Result};

/// Parameters for creating a pull request.
#[derive(Debug, Clone)]
pub struct CreatePrParams {
    pub owner: String,
    pub repo: String,
    pub title: String,
    pub body: String,
    pub head: String,
    pub base: Option<String>,
    pub draft: bool,
}

/// PR state from GitHub API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

/// PR information from GitHub API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub state: PrState,
    pub url: String,
}

/// Parameters for updating a pull request.
#[derive(Debug, Clone)]
pub struct UpdatePrParams {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub body: String,
}

/// Query parameter for batch PR lookup across repos and branches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchPrQuery {
    pub owner: String,
    pub repo: String,
    pub branch: String,
}

impl BranchPrQuery {
    /// Returns the map key used for this query.
    pub fn key(&self) -> (String, String, String) {
        (self.owner.clone(), self.repo.clone(), self.branch.clone())
    }
}

/// Trait for pull request operations.
pub trait PrClient: Send + Sync {
    /// Create a pull request and return its URL.
    async fn create_pull_request(&self, params: CreatePrParams) -> Result<String>;

    /// Update a pull request's title and body, return its URL.
    async fn update_pull_request(&self, params: UpdatePrParams) -> Result<String>;

    /// Get PR state for a branch. Returns None if no PR exists.
    async fn get_pr_for_branch(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Option<PrInfo>>;
}

/// REST API response for PR create/update/list.
#[derive(Debug, Deserialize)]
struct PrResponse {
    number: u64,
    title: Option<String>,
    state: Option<String>,
    html_url: Option<String>,
    merged_at: Option<String>,
}

impl PrClient for GitHubClient {
    async fn create_pull_request(&self, params: CreatePrParams) -> Result<String> {
        // If base is not specified, resolve it via the GitHub API (see find_base_branch).
        let base = match &params.base {
            Some(b) => b.clone(),
            None => crate::infra::git::find_base_branch(&params.owner, &params.repo, self).await,
        };

        let mut payload = serde_json::json!({
            "title": params.title,
            "body": params.body,
            "head": params.head,
            "base": base,
        });

        if params.draft {
            payload["draft"] = serde_json::json!(true);
        }

        let route = format!("/repos/{}/{}/pulls", params.owner, params.repo);
        let pr: PrResponse = self.rest_post(&route, &payload).await?;

        pr.html_url.ok_or_else(|| GitHubError::MissingPrUrl.into())
    }

    async fn update_pull_request(&self, params: UpdatePrParams) -> Result<String> {
        let route = format!(
            "/repos/{}/{}/pulls/{}",
            params.owner, params.repo, params.number
        );
        let payload = serde_json::json!({
            "title": params.title,
            "body": params.body,
        });

        let pr: PrResponse = self.rest_patch(&route, &payload).await?;

        pr.html_url.ok_or_else(|| GitHubError::MissingPrUrl.into())
    }

    async fn get_pr_for_branch(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Option<PrInfo>> {
        let route = format!("/repos/{owner}/{repo}/pulls");
        let head = format!("{owner}:{branch}");
        let pulls: Vec<PrResponse> = self
            .rest_get_with_query(&route, &[("head", &head), ("state", "all")])
            .await?;

        let Some(pr) = pulls.into_iter().next() else {
            return Ok(None);
        };

        let state = if pr.merged_at.is_some() {
            PrState::Merged
        } else {
            match pr.state.as_deref() {
                Some("open") => PrState::Open,
                Some("closed") => PrState::Closed,
                _ => PrState::Closed,
            }
        };

        let url = pr.html_url.unwrap_or_default();

        Ok(Some(PrInfo {
            number: pr.number,
            title: pr.title.unwrap_or_default(),
            state,
            url,
        }))
    }
}

/// Maximum number of branch queries per single GraphQL request.
/// GitHub GraphQL API has complexity limits; 50 branches keeps us well within bounds.
const BATCH_SIZE: usize = 50;

impl GitHubClient {
    /// Fetch PR status for multiple repo/branch combinations in a single GraphQL call.
    ///
    /// Uses aliased GraphQL queries to batch multiple repository+branch lookups,
    /// avoiding N+1 REST API calls when checking many branches at once.
    /// Returns a map from (owner, repo, branch) to the most recent PR info (if any).
    pub async fn get_prs_for_branches_batch(
        &self,
        queries: &[BranchPrQuery],
    ) -> Result<HashMap<(String, String, String), Option<PrInfo>>> {
        if queries.is_empty() {
            return Ok(HashMap::new());
        }

        let mut all_results: HashMap<(String, String, String), Option<PrInfo>> = HashMap::new();

        // Split into chunks to stay within GraphQL complexity limits
        for chunk in queries.chunks(BATCH_SIZE) {
            let chunk_results = self.execute_batch_query(chunk).await?;
            all_results.extend(chunk_results);
        }

        Ok(all_results)
    }

    /// Build and execute a single batched GraphQL query for a chunk of branch queries.
    ///
    /// Uses GraphQL variables for all user-supplied strings (owner, repo, branch)
    /// to avoid injection risks from string interpolation.
    async fn execute_batch_query(
        &self,
        queries: &[BranchPrQuery],
    ) -> Result<HashMap<(String, String, String), Option<PrInfo>>> {
        // Group queries by (owner, repo) so each repo appears once in the GraphQL query
        let mut repo_branches: HashMap<(String, String), Vec<(usize, String)>> = HashMap::new();
        for (i, q) in queries.iter().enumerate() {
            repo_branches
                .entry((q.owner.clone(), q.repo.clone()))
                .or_default()
                .push((i, q.branch.clone()));
        }

        // Build the GraphQL query with aliases and variables
        let mut variable_defs = Vec::new();
        let mut variables = serde_json::Map::new();
        let mut query_parts = Vec::new();
        // Track alias -> (owner, repo, branch) for response parsing
        let mut alias_map: HashMap<String, HashMap<String, (String, String, String)>> =
            HashMap::new();

        for (repo_idx, ((owner, repo), branches)) in repo_branches.iter().enumerate() {
            let repo_alias = format!("repo{repo_idx}");
            let owner_var = format!("owner_{repo_idx}");
            let repo_var = format!("repoName_{repo_idx}");

            variable_defs.push(format!("${owner_var}: String!"));
            variable_defs.push(format!("${repo_var}: String!"));
            variables.insert(owner_var.clone(), serde_json::Value::String(owner.clone()));
            variables.insert(repo_var.clone(), serde_json::Value::String(repo.clone()));

            let mut branch_parts = Vec::new();
            let mut branch_alias_map = HashMap::new();

            for (branch_idx, (_, branch)) in branches.iter().enumerate() {
                let branch_alias = format!("branch{branch_idx}");
                let branch_var = format!("branch{repo_idx}_{branch_idx}");

                variable_defs.push(format!("${branch_var}: String!"));
                variables.insert(
                    branch_var.clone(),
                    serde_json::Value::String(branch.clone()),
                );

                branch_parts.push(format!(
                    "{branch_alias}: pullRequests(headRefName: ${branch_var}, states: [OPEN, CLOSED, MERGED], first: 1, orderBy: {{field: CREATED_AT, direction: DESC}}) {{ nodes {{ number title state url mergedAt }} }}"
                ));
                branch_alias_map
                    .insert(branch_alias, (owner.clone(), repo.clone(), branch.clone()));
            }

            let branch_query = branch_parts.join("\n    ");
            query_parts.push(format!(
                "{repo_alias}: repository(owner: ${owner_var}, name: ${repo_var}) {{\n    {branch_query}\n  }}"
            ));
            alias_map.insert(repo_alias, branch_alias_map);
        }

        let query = format!(
            "query({}) {{\n  {}\n}}",
            variable_defs.join(", "),
            query_parts.join("\n  ")
        );

        // Execute and parse. Use serde_json::Value since response structure is dynamic.
        let response: serde_json::Value = self
            .graphql::<serde_json::Value>(&query, serde_json::Value::Object(variables))
            .await?;

        let mut results: HashMap<(String, String, String), Option<PrInfo>> = HashMap::new();

        for (repo_alias, branch_aliases) in &alias_map {
            let repo_data = response.get(repo_alias);

            for (branch_alias, key) in branch_aliases {
                let pr_info = repo_data
                    .and_then(|r| r.get(branch_alias))
                    .and_then(|pr_connection| pr_connection.get("nodes"))
                    .and_then(|nodes| nodes.as_array())
                    .and_then(|nodes| nodes.first())
                    .and_then(parse_pr_node);

                results.insert(key.clone(), pr_info);
            }
        }

        // Ensure all queried branches have an entry (even if repo was missing from response)
        for q in queries {
            results.entry(q.key()).or_insert(None);
        }

        Ok(results)
    }
}

/// Parse a single PR node from the GraphQL response into PrInfo.
fn parse_pr_node(node: &serde_json::Value) -> Option<PrInfo> {
    let number = node.get("number")?.as_u64()?;
    let title = node.get("title")?.as_str()?.to_string();
    let state_str = node.get("state")?.as_str()?;
    let url = node.get("url")?.as_str()?.to_string();
    let merged_at = node.get("mergedAt").and_then(|v| v.as_str());

    let state = if merged_at.is_some() {
        PrState::Merged
    } else {
        match state_str {
            "OPEN" => PrState::Open,
            "CLOSED" => PrState::Closed,
            "MERGED" => PrState::Merged,
            _ => PrState::Closed,
        }
    };

    Some(PrInfo {
        number,
        title,
        state,
        url,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rstest::rstest;
    use serde_json::json;

    use super::{BranchPrQuery, PrInfo, PrState, parse_pr_node};

    #[tokio::test]
    async fn batch_lookup_requests_and_returns_pull_request_title() {
        let mock = crate::infra::github::mock::GitHubMockServer::start().await;
        mock.graphql_batch_pull_request(json!({
            "number": 42,
            "title": "A merged change",
            "state": "MERGED",
            "url": "https://github.com/owner/repo/pull/42",
            "mergedAt": "2025-01-01T00:00:00Z",
        }))
        .await;

        let client = mock.client();
        let query = BranchPrQuery {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            branch: "feature".to_string(),
        };
        let actual = client
            .get_prs_for_branches_batch(std::slice::from_ref(&query))
            .await
            .expect("batch lookup");

        assert_eq!(
            actual,
            HashMap::from([(
                query.key(),
                Some(PrInfo {
                    number: 42,
                    title: "A merged change".to_string(),
                    state: PrState::Merged,
                    url: "https://github.com/owner/repo/pull/42".to_string(),
                }),
            )]),
        );
    }

    #[rstest]
    #[case::merged(
        json!({
            "number": 42,
            "title": "A merged change",
            "state": "MERGED",
            "url": "https://github.com/owner/repo/pull/42",
            "mergedAt": "2026-10-04T00:00:00Z",
        }),
        Some(PrInfo {
            number: 42,
            title: "A merged change".to_string(),
            state: PrState::Merged,
            url: "https://github.com/owner/repo/pull/42".to_string(),
        }),
    )]
    #[case::closed_unmerged(
        json!({
            "number": 43,
            "title": "A closed change",
            "state": "CLOSED",
            "url": "https://github.com/owner/repo/pull/43",
            "mergedAt": null,
        }),
        Some(PrInfo {
            number: 43,
            title: "A closed change".to_string(),
            state: PrState::Closed,
            url: "https://github.com/owner/repo/pull/43".to_string(),
        }),
    )]
    fn parses_pull_request_title_and_state(
        #[case] input: serde_json::Value,
        #[case] expected: Option<PrInfo>,
    ) {
        assert_eq!(parse_pr_node(&input), expected);
    }
}
