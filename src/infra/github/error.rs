//! GitHub API error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitHubError {
    #[error("Failed to get GitHub token: {0}")]
    TokenError(String),

    #[error("{0}")]
    ApiError(String),

    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("PR created but no URL in response")]
    MissingPrUrl,

    #[error("GraphQL error: {0}")]
    GraphQLError(String),
}

pub type Result<T> = anyhow::Result<T>;

/// Format error details from GitHub API errors array.
/// Returns a formatted string like "[field1 is code1, field2 is code2]" or empty string.
fn format_error_details(errors: &[serde_json::Value]) -> String {
    let error_details: Vec<String> = errors
        .iter()
        .filter_map(|e| {
            let field = e.get("field").and_then(|v| v.as_str());
            let code = e.get("code").and_then(|v| v.as_str());
            match (field, code) {
                (Some(f), Some(c)) => Some(format!("{f} is {c}")),
                (Some(f), None) => Some(f.to_string()),
                (None, Some(c)) => Some(c.to_string()),
                (None, None) => None,
            }
        })
        .collect();

    if error_details.is_empty() {
        String::new()
    } else {
        format!(" [{}]", error_details.join(", "))
    }
}

/// Build an ApiError from a GitHub API error response body.
pub(crate) fn api_error_from_response(status: u16, body: &serde_json::Value) -> GitHubError {
    let message = body
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown error");

    let mut msg = format!("GitHub API error: {message} (HTTP {status})");

    if let Some(errors) = body.get("errors").and_then(|v| v.as_array()) {
        msg.push_str(&format_error_details(errors));
    }

    GitHubError::ApiError(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    #[case::field_and_code(
        vec![json!({"field": "base", "code": "invalid"})],
        " [base is invalid]"
    )]
    #[case::multiple_errors(
        vec![
            json!({"field": "title", "code": "missing"}),
            json!({"field": "body", "code": "too_long"})
        ],
        " [title is missing, body is too_long]"
    )]
    #[case::field_only(
        vec![json!({"field": "name"})],
        " [name]"
    )]
    #[case::code_only(
        vec![json!({"code": "custom"})],
        " [custom]"
    )]
    #[case::empty_object(
        vec![json!({})],
        ""
    )]
    #[case::empty_array(
        vec![],
        ""
    )]
    #[case::mixed_valid_and_invalid(
        vec![json!({"field": "a", "code": "b"}), json!({}), json!({"field": "c"})],
        " [a is b, c]"
    )]
    fn test_format_error_details(#[case] errors: Vec<serde_json::Value>, #[case] expected: &str) {
        assert_eq!(format_error_details(&errors), expected);
    }

    #[test]
    fn test_github_error_token_error_display() {
        let err = GitHubError::TokenError("test error".to_string());
        assert_eq!(err.to_string(), "Failed to get GitHub token: test error");
    }

    #[test]
    fn test_github_error_missing_pr_url_display() {
        let err = GitHubError::MissingPrUrl;
        assert_eq!(err.to_string(), "PR created but no URL in response");
    }

    #[test]
    fn test_api_error_from_response_with_errors() {
        let body = json!({
            "message": "Validation Failed",
            "errors": [{"field": "title", "code": "missing"}]
        });
        let err = api_error_from_response(422, &body);
        assert_eq!(
            err.to_string(),
            "GitHub API error: Validation Failed (HTTP 422) [title is missing]"
        );
    }

    #[test]
    fn test_api_error_from_response_without_errors() {
        let body = json!({"message": "Not Found"});
        let err = api_error_from_response(404, &body);
        assert_eq!(err.to_string(), "GitHub API error: Not Found (HTTP 404)");
    }
}
