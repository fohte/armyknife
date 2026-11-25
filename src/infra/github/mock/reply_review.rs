use super::GitHubMockServer;
use serde_json::json;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, ResponseTemplate};

impl GitHubMockServer {
    pub async fn graphql_reply_review_status(
        &self,
        state: &str,
        merged_at: Option<&str>,
        end_cursor: Option<&str>,
        has_next_page: bool,
        thread_nodes: Vec<serde_json::Value>,
    ) {
        self.graphql_reply_review_status_page(
            None,
            state,
            merged_at,
            end_cursor,
            has_next_page,
            thread_nodes,
        )
        .await;
    }

    pub async fn graphql_reply_review_status_page(
        &self,
        request_cursor: Option<&str>,
        state: &str,
        merged_at: Option<&str>,
        end_cursor: Option<&str>,
        has_next_page: bool,
        thread_nodes: Vec<serde_json::Value>,
    ) {
        let cursor = request_cursor.map_or_else(
            || "\"threadCursor\":null".to_string(),
            |cursor| format!("\"threadCursor\":\"{cursor}\""),
        );

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains(
                "reviewThreads(first: 100, after: $threadCursor)",
            ))
            .and(body_string_contains(cursor))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "repository": {
                        "pullRequest": {
                            "state": state,
                            "mergedAt": merged_at,
                            "reviewThreads": {
                                "pageInfo": {
                                    "hasNextPage": has_next_page,
                                    "endCursor": end_cursor
                                },
                                "nodes": thread_nodes
                            }
                        }
                    }
                }
            })))
            .mount(&self.server)
            .await;
    }
}
