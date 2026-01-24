//! GitHub API error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitHubError {
    #[error("Failed to get GitHub token: {0}")]
    TokenError(String),

    #[error("{}", format_octocrab_error(.0))]
    ApiError(#[from] octocrab::Error),

    #[error("PR created but no URL in response")]
    MissingPrUrl,

    #[error("GraphQL error: {0}")]
    GraphQLError(String),
}

/// Format octocrab::Error to extract detailed error information from GitHub API responses.
fn format_octocrab_error(err: &octocrab::Error) -> String {
    match err {
        octocrab::Error::GitHub { source, .. } => {
            let mut msg = format!(
                "GitHub API error: {} (HTTP {})",
                source.message,
                source.status_code.as_u16()
            );

            // Add detailed error information if available
            if let Some(errors) = &source.errors {
                msg.push_str(&format_error_details(errors));
            }

            msg
        }
        // For other error types, use the default Display implementation
        _ => format!("GitHub API error: {err}"),
    }
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
}
