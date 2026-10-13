use std::collections::HashMap;

use super::client::GitHubClient;
use super::error::Result;
use super::pr::{PR_PROGRESS_FIELDS, PrProgressInfo, parse_pr_progress_node};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// Identifies a pull request by repository and number.
pub struct PrNumberQuery {
    /// GitHub repository owner.
    pub owner: String,
    /// GitHub repository name.
    pub repo: String,
    /// Pull request number.
    pub number: u64,
}

impl PrNumberQuery {
    /// Returns the map key used for this query.
    pub fn key(&self) -> (String, String, u64) {
        (self.owner.clone(), self.repo.clone(), self.number)
    }
}

const BATCH_SIZE: usize = 50;

impl GitHubClient {
    /// Fetches pull request details and CI status for multiple PR numbers.
    pub async fn get_pr_progress_for_numbers_batch(
        &self,
        queries: &[PrNumberQuery],
    ) -> Result<HashMap<(String, String, u64), Option<PrProgressInfo>>> {
        let mut all_results = HashMap::new();
        for chunk in queries.chunks(BATCH_SIZE) {
            all_results.extend(self.execute_number_batch_query(chunk).await?);
        }
        Ok(all_results)
    }

    async fn execute_number_batch_query(
        &self,
        queries: &[PrNumberQuery],
    ) -> Result<HashMap<(String, String, u64), Option<PrProgressInfo>>> {
        if queries.is_empty() {
            return Ok(HashMap::new());
        }

        let mut repo_queries: HashMap<(String, String), Vec<(usize, u64)>> = HashMap::new();
        for (index, query) in queries.iter().enumerate() {
            repo_queries
                .entry((query.owner.clone(), query.repo.clone()))
                .or_default()
                .push((index, query.number));
        }

        let mut variable_defs = Vec::new();
        let mut variables = serde_json::Map::new();
        let mut query_parts = Vec::new();
        let mut aliases: HashMap<String, HashMap<String, PrNumberQuery>> = HashMap::new();

        for (repo_index, ((owner, repo), pull_requests)) in repo_queries.iter().enumerate() {
            let repo_alias = format!("repo{repo_index}");
            let owner_var = format!("owner_{repo_index}");
            let repo_var = format!("repoName_{repo_index}");
            variable_defs.push(format!("${owner_var}: String!"));
            variable_defs.push(format!("${repo_var}: String!"));
            variables.insert(owner_var.clone(), serde_json::Value::String(owner.clone()));
            variables.insert(repo_var.clone(), serde_json::Value::String(repo.clone()));

            let mut pull_request_parts = Vec::new();
            let mut repo_aliases = HashMap::new();
            for (pull_request_index, (query_index, number)) in pull_requests.iter().enumerate() {
                let pull_request_alias = format!("pr{pull_request_index}");
                let number_var = format!("number_{repo_index}_{pull_request_index}");
                variable_defs.push(format!("${number_var}: Int!"));
                variables.insert(
                    number_var.clone(),
                    serde_json::Value::Number((*number).into()),
                );
                pull_request_parts.push(format!(
                    "{pull_request_alias}: pullRequest(number: ${number_var}) {{ {PR_PROGRESS_FIELDS} }}"
                ));
                repo_aliases.insert(pull_request_alias, queries[*query_index].clone());
            }

            query_parts.push(format!(
                "{}: repository(owner: {}{}, name: {}{}) {{ {} }}",
                repo_alias,
                "$",
                owner_var,
                "$",
                repo_var,
                pull_request_parts.join("\n    ")
            ));
            aliases.insert(repo_alias, repo_aliases);
        }

        let query = format!(
            "query({}) {{ {} }}",
            variable_defs.join(", "),
            query_parts.join("\n  ")
        );
        let response: serde_json::Value = self
            .graphql_with_partial_data(&query, serde_json::Value::Object(variables))
            .await?;

        let mut results = HashMap::new();
        for (repo_alias, pull_request_aliases) in aliases {
            for (pull_request_alias, query) in pull_request_aliases {
                let progress = response
                    .get(&repo_alias)
                    .and_then(|repo| repo.get(&pull_request_alias))
                    .and_then(parse_pr_progress_node);
                results.insert(query.key(), progress);
            }
        }

        for query in queries {
            results.entry(query.key()).or_insert(None);
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;
    use crate::infra::github::{PrCiState, PrInfo, PrState};

    #[tokio::test]
    async fn looks_up_pr_status_by_number() {
        let mock = crate::infra::github::mock::GitHubMockServer::start().await;
        mock.graphql_pull_request_by_number_with_ci(json!({
            "number": 17,
            "title": "Add a sample feature",
            "state": "OPEN",
            "url": "https://github.com/sample-org/sample-project/pull/17",
            "mergedAt": null,
            "commits": {
                "nodes": [{
                    "commit": {
                        "statusCheckRollup": { "state": "SUCCESS" }
                    }
                }]
            }
        }))
        .await;

        let query = PrNumberQuery {
            owner: "sample-org".to_string(),
            repo: "sample-project".to_string(),
            number: 17,
        };

        assert_eq!(
            mock.client()
                .get_pr_progress_for_numbers_batch(std::slice::from_ref(&query))
                .await
                .unwrap(),
            HashMap::from([(
                query.key(),
                Some(PrProgressInfo {
                    pull_request: PrInfo {
                        number: 17,
                        title: "Add a sample feature".to_string(),
                        state: PrState::Open,
                        url: "https://github.com/sample-org/sample-project/pull/17".to_string(),
                    },
                    ci_state: PrCiState::Successful,
                }),
            )]),
        );
    }

    #[tokio::test]
    async fn keeps_valid_pr_when_another_number_is_missing() {
        let mock = crate::infra::github::mock::GitHubMockServer::start().await;
        mock.graphql_pull_requests_by_number_with_ci(
            vec![
                None,
                Some(json!({
                    "number": 18,
                    "title": "Add another sample feature",
                    "state": "OPEN",
                    "url": "https://github.com/example-org/example-project/pull/18",
                    "mergedAt": null,
                    "commits": {
                        "nodes": [{
                            "commit": {
                                "statusCheckRollup": { "state": "SUCCESS" }
                            }
                        }]
                    }
                })),
            ],
            &[0],
        )
        .await;

        let missing_query = PrNumberQuery {
            owner: "example-org".to_string(),
            repo: "example-project".to_string(),
            number: 17,
        };
        let valid_query = PrNumberQuery {
            number: 18,
            ..missing_query.clone()
        };

        assert_eq!(
            mock.client()
                .get_pr_progress_for_numbers_batch(&[missing_query.clone(), valid_query.clone(),])
                .await
                .unwrap(),
            HashMap::from([
                (missing_query.key(), None),
                (
                    valid_query.key(),
                    Some(PrProgressInfo {
                        pull_request: PrInfo {
                            number: 18,
                            title: "Add another sample feature".to_string(),
                            state: PrState::Open,
                            url: "https://github.com/example-org/example-project/pull/18"
                                .to_string(),
                        },
                        ci_state: PrCiState::Successful,
                    }),
                ),
            ]),
        );
    }

    #[tokio::test]
    async fn empty_batch_returns_no_results() {
        let mock = crate::infra::github::mock::GitHubMockServer::start().await;

        assert_eq!(
            mock.client()
                .get_pr_progress_for_numbers_batch(&[])
                .await
                .unwrap(),
            HashMap::new(),
        );
    }
}
