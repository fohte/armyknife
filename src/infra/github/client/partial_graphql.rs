use super::super::error::{GitHubError, Result};
use super::{GitHubClient, GraphQLResponse, check_response};

impl GitHubClient {
    /// Execute a GraphQL query and return any available data, even when some
    /// fields reported errors. Callers must handle null fields individually.
    pub(in crate::infra::github) async fn graphql_with_partial_data<
        T: serde::de::DeserializeOwned,
    >(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<T> {
        let body = serde_json::json!({
            "query": query,
            "variables": variables,
        });
        let response = self.http.post(&self.graphql_url).json(&body).send().await?;
        let response: GraphQLResponse<T> = check_response(response).await?;

        response
            .data
            .ok_or_else(|| GitHubError::GraphQLError("No data in response".to_string()).into())
    }
}
