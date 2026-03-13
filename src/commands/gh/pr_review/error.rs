use thiserror::Error;

use crate::infra::git;

/// Legacy error type preserved for the check subcommand.
#[derive(Error, Debug)]
pub enum CheckPrReviewError {
    #[error("Git error: {0}")]
    GitError(#[from] git::GitError),

    #[error("GitHub API error: {0}")]
    GitHubError(#[from] crate::infra::github::GitHubError),

    #[error("GraphQL API error: {0}")]
    GraphQLError(String),

    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Review [{0}] not found. Run without --review to see available reviews.")]
    ReviewNotFound(usize),
}

/// Unified error type for pr-review reply operations.
#[derive(Error, Debug)]
pub enum PrReviewError {
    #[error("GitHub API error: {url} returned {status}")]
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "will be used when API error handling is wired up")
    )]
    GitHubApiError { url: String, status: u16 },

    #[error("PR #{number} not found in {owner}/{repo}")]
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "will be used when API error handling is wired up")
    )]
    PrNotFound {
        owner: String,
        repo: String,
        number: u64,
    },

    #[error("GraphQL error: {message}")]
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "will be used when API error handling is wired up")
    )]
    GraphQLError { message: String },

    #[error("Failed to post reply to thread {thread_id}: {details}")]
    ReplyPostFailed { thread_id: String, details: String },

    #[error("Failed to resolve thread {thread_id}: {details}")]
    ResolveFailed { thread_id: String, details: String },

    #[error("Parse error at line {line}: {details}")]
    ThreadParseError { line: usize, details: String },

    #[error("Invalid frontmatter: {details}")]
    FrontmatterParseError { details: String },

    #[error("{}", format_conflict_error(*.count, details))]
    ConflictDetected { count: usize, details: String },

    #[error("Local changes detected. Use --force to overwrite")]
    LocalChangesExist,

    #[error("No pulled data found. Run 'reply pull' first")]
    NoPulledData,

    #[error("Failed to read {path}: {details}")]
    StorageReadError { path: String, details: String },

    #[error("Failed to write {path}: {details}")]
    StorageWriteError { path: String, details: String },

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

fn format_conflict_error(count: usize, details: &str) -> String {
    format!(
        "Conflict detected in {count} thread(s):\n\
         {details}\n\
         Use --force to override, or re-pull"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::github_api_error(
        PrReviewError::GitHubApiError { url: "https://api.github.com/repos/fohte/armyknife".to_string(), status: 403 },
        "GitHub API error: https://api.github.com/repos/fohte/armyknife returned 403"
    )]
    #[case::graphql_error(
        PrReviewError::GraphQLError { message: "rate limited".to_string() },
        "GraphQL error: rate limited"
    )]
    #[case::pr_not_found(
        PrReviewError::PrNotFound { owner: "fohte".to_string(), repo: "armyknife".to_string(), number: 42 },
        "PR #42 not found in fohte/armyknife"
    )]
    #[case::local_changes(
        PrReviewError::LocalChangesExist,
        "Local changes detected. Use --force to overwrite"
    )]
    #[case::no_pulled_data(
        PrReviewError::NoPulledData,
        "No pulled data found. Run 'reply pull' first"
    )]
    #[case::parse_error(
        PrReviewError::ThreadParseError { line: 10, details: "unexpected token".to_string() },
        "Parse error at line 10: unexpected token"
    )]
    #[case::frontmatter_error(
        PrReviewError::FrontmatterParseError { details: "missing pr field".to_string() },
        "Invalid frontmatter: missing pr field"
    )]
    #[case::reply_post_failed(
        PrReviewError::ReplyPostFailed { thread_id: "abc".to_string(), details: "403 Forbidden".to_string() },
        "Failed to post reply to thread abc: 403 Forbidden"
    )]
    #[case::resolve_failed(
        PrReviewError::ResolveFailed { thread_id: "def".to_string(), details: "not found".to_string() },
        "Failed to resolve thread def: not found"
    )]
    fn test_error_display(#[case] error: PrReviewError, #[case] expected: &str) {
        assert_eq!(error.to_string(), expected);
    }
}
