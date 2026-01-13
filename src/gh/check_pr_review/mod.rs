mod api;
mod format;
mod models;

use crate::git;
use clap::Args;
use thiserror::Error;

pub use api::fetch_pr_data;

#[derive(Error, Debug)]
pub enum CheckPrReviewError {
    #[error("Failed to get repository info: {0}")]
    RepoInfoError(String),

    #[error("Git error: {0}")]
    GitError(#[from] git::GitError),

    #[error("GitHub API error: {0}")]
    GitHubError(#[from] crate::github::GitHubError),

    #[error("GraphQL API error: {0}")]
    GraphQLError(String),

    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Review [{0}] not found. Run without --review to see available reviews.")]
    ReviewNotFound(usize),
}

pub type Result<T> = std::result::Result<T, CheckPrReviewError>;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CheckPrReviewArgs {
    /// PR number
    pub pr_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,

    /// Include resolved threads
    #[arg(short = 'a', long = "all")]
    pub include_resolved: bool,

    /// Show details for review number N
    #[arg(short = 'r', long = "review")]
    pub show_review: Option<usize>,

    /// Show all details
    #[arg(short = 'f', long = "full")]
    pub full_mode: bool,

    /// Expand HTML details blocks
    #[arg(short = 'd', long = "open-details")]
    pub open_details: bool,
}

pub async fn run(args: &CheckPrReviewArgs) -> Result<()> {
    let (owner, repo) = get_repo_owner_and_name(args.repo.as_deref())?;

    let pr_data = fetch_pr_data(&owner, &repo, args.pr_number, args.include_resolved).await?;

    if pr_data.reviews.is_empty() && pr_data.threads.is_empty() {
        println!("No review comments found.");
        return Ok(());
    }

    let options = format::FormatOptions {
        open_details: args.open_details,
        skip_delta: false,
    };

    match (args.show_review, args.full_mode) {
        (Some(review_num), _) => {
            format::print_review_details(&pr_data, review_num, &options)?;
        }
        (_, true) => {
            format::print_full(&pr_data, &options);
        }
        _ => {
            format::print_summary(&pr_data);
        }
    }

    Ok(())
}

fn get_repo_owner_and_name(repo_arg: Option<&str>) -> Result<(String, String)> {
    if let Some(repo) = repo_arg {
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() == 2 {
            return Ok((parts[0].to_string(), parts[1].to_string()));
        }
        return Err(CheckPrReviewError::RepoInfoError(format!(
            "Invalid repository format: {repo}. Expected owner/repo"
        )));
    }

    // Get from current repo using git2
    let repo = git::open_repo()?;
    let (owner, name) = git::github_owner_and_repo(&repo)?;
    Ok((owner, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::valid("owner/repo", "owner", "repo")]
    #[case::with_dashes("my-org/my-repo", "my-org", "my-repo")]
    #[case::with_numbers("user123/project456", "user123", "project456")]
    fn test_get_repo_owner_and_name_with_arg(
        #[case] input: &str,
        #[case] expected_owner: &str,
        #[case] expected_repo: &str,
    ) {
        let (owner, repo) = get_repo_owner_and_name(Some(input)).unwrap();
        assert_eq!(owner, expected_owner);
        assert_eq!(repo, expected_repo);
    }

    #[rstest]
    #[case::no_slash("invalid")]
    #[case::too_many_slashes("a/b/c")]
    #[case::empty("")]
    fn test_get_repo_owner_and_name_invalid(#[case] input: &str) {
        let result = get_repo_owner_and_name(Some(input));
        assert!(result.is_err());
        assert!(matches!(result, Err(CheckPrReviewError::RepoInfoError(_))));
    }
}
