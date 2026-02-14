//! Pull request operations.

use std::collections::HashMap;

use super::client::OctocrabClient;
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
#[derive(Debug, Clone)]
pub struct PrInfo {
    pub number: u64,
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
#[derive(Debug, Clone)]
pub struct BranchPrQuery {
    pub owner: String,
    pub repo: String,
    pub branch: String,
}

/// Trait for pull request operations.
#[async_trait::async_trait]
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

    /// Open a URL in the default browser.
    fn open_in_browser(&self, url: &str);
}

#[async_trait::async_trait]
impl PrClient for OctocrabClient {
    async fn create_pull_request(&self, params: CreatePrParams) -> Result<String> {
        let pulls = self.client.pulls(&params.owner, &params.repo);

        // If base is not specified, find the base branch from local git info or GitHub API
        let base = match &params.base {
            Some(b) => b.clone(),
            None => crate::infra::git::find_base_branch(&params.owner, &params.repo, self).await,
        };

        let pr = if params.draft {
            pulls
                .create(&params.title, &params.head, &base)
                .body(&params.body)
                .draft(Some(true))
                .send()
                .await?
        } else {
            pulls
                .create(&params.title, &params.head, &base)
                .body(&params.body)
                .send()
                .await?
        };

        pr.html_url
            .map(|u| u.to_string())
            .ok_or_else(|| GitHubError::MissingPrUrl.into())
    }

    async fn update_pull_request(&self, params: UpdatePrParams) -> Result<String> {
        let pr = self
            .client
            .pulls(&params.owner, &params.repo)
            .update(params.number)
            .title(&params.title)
            .body(&params.body)
            .send()
            .await?;

        pr.html_url
            .map(|u| u.to_string())
            .ok_or_else(|| GitHubError::MissingPrUrl.into())
    }

    async fn get_pr_for_branch(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Option<PrInfo>> {
        // Search for PRs with this head branch
        let pulls = self
            .client
            .pulls(owner, repo)
            .list()
            .head(format!("{owner}:{branch}"))
            .state(octocrab::params::State::All)
            .send()
            .await?;

        // Get the first (most recent) PR for this branch
        let Some(pr) = pulls.items.into_iter().next() else {
            return Ok(None);
        };

        let state = if pr.merged_at.is_some() {
            PrState::Merged
        } else {
            match pr.state {
                Some(octocrab::models::IssueState::Open) => PrState::Open,
                Some(octocrab::models::IssueState::Closed) => PrState::Closed,
                _ => PrState::Closed,
            }
        };

        let url = pr.html_url.map(|u| u.to_string()).unwrap_or_default();

        Ok(Some(PrInfo {
            number: pr.number,
            state,
            url,
        }))
    }

    fn open_in_browser(&self, url: &str) {
        // Skip in test mode to prevent browser opening during tests
        if cfg!(test) {
            return;
        }
        let _ = open::that(url);
    }
}

/// Maximum number of branch queries per single GraphQL request.
/// GitHub GraphQL API has complexity limits; 50 branches keeps us well within bounds.
const BATCH_SIZE: usize = 50;

impl OctocrabClient {
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

        // Build the GraphQL query with aliases
        let mut query_parts = Vec::new();
        // Track alias -> (owner, repo, branch) for response parsing
        let mut alias_map: HashMap<String, HashMap<String, (String, String, String)>> =
            HashMap::new();

        for (repo_idx, ((owner, repo), branches)) in repo_branches.iter().enumerate() {
            let repo_alias = format!("repo{repo_idx}");
            let mut branch_parts = Vec::new();
            let mut branch_alias_map = HashMap::new();

            for (branch_idx, (_, branch)) in branches.iter().enumerate() {
                let branch_alias = format!("branch{branch_idx}");
                let escaped_branch = escape_graphql_string(branch);
                branch_parts.push(format!(
                    "{branch_alias}: pullRequests(headRefName: \"{escaped_branch}\", states: [OPEN, CLOSED, MERGED], first: 1, orderBy: {{field: CREATED_AT, direction: DESC}}) {{ nodes {{ number state url mergedAt }} }}"
                ));
                branch_alias_map
                    .insert(branch_alias, (owner.clone(), repo.clone(), branch.clone()));
            }

            let branch_query = branch_parts.join("\n    ");
            let escaped_owner = escape_graphql_string(owner);
            let escaped_repo = escape_graphql_string(repo);
            query_parts.push(format!(
                "{repo_alias}: repository(owner: \"{escaped_owner}\", name: \"{escaped_repo}\") {{\n    {branch_query}\n  }}"
            ));
            alias_map.insert(repo_alias, branch_alias_map);
        }

        let query = format!("{{\n  {}\n}}", query_parts.join("\n  "));

        // Execute and parse. Use serde_json::Value since response structure is dynamic.
        let response: serde_json::Value = match self
            .graphql::<serde_json::Value>(&query, serde_json::json!({}))
            .await
        {
            Ok(data) => data,
            Err(_) => {
                // On total failure (e.g., one repo not found causes GraphQL errors),
                // return all branches as None so callers can fall back to slower paths
                return Ok(queries
                    .iter()
                    .map(|q| ((q.owner.clone(), q.repo.clone(), q.branch.clone()), None))
                    .collect());
            }
        };

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
            let key = (q.owner.clone(), q.repo.clone(), q.branch.clone());
            results.entry(key).or_insert(None);
        }

        Ok(results)
    }
}

/// Escape a string for use inside a GraphQL double-quoted string literal.
fn escape_graphql_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Parse a single PR node from the GraphQL response into PrInfo.
fn parse_pr_node(node: &serde_json::Value) -> Option<PrInfo> {
    let number = node.get("number")?.as_u64()?;
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

    Some(PrInfo { number, state, url })
}
