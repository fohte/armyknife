//! GitHub API error types.

use std::fmt::Write;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitHubError {
    #[error("Failed to get GitHub token: {0}")]
    TokenError(String),

    #[error("{}", format_octocrab_error(.0))]
    ApiError(#[from] octocrab::Error),

    #[error("PR created but no URL in response")]
    MissingPrUrl,
}

/// Format octocrab::Error to extract detailed error information from GitHub API responses.
fn format_octocrab_error(err: &octocrab::Error) -> String {
    match err {
        octocrab::Error::GitHub { source, .. } => {
            let mut msg = format!("GitHub API error: {}", source.message);

            // Add HTTP status code
            write!(&mut msg, " (HTTP {})", source.status_code.as_u16()).unwrap();

            // Add detailed error information if available
            if let Some(errors) = &source.errors {
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

                if !error_details.is_empty() {
                    write!(&mut msg, " [{}]", error_details.join(", ")).unwrap();
                }
            }

            msg
        }
        // For other error types, use the default Display implementation
        _ => format!("GitHub API error: {err}"),
    }
}

pub type Result<T> = std::result::Result<T, GitHubError>;
