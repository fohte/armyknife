mod api;
mod format;
mod models;

use clap::Args;
use thiserror::Error;

pub use api::fetch_pr_data;

#[derive(Error, Debug)]
pub enum CheckPrReviewError {
    #[error("Failed to get repository info: {0}")]
    RepoInfoError(String),

    #[error("GraphQL API error: {0}")]
    GraphQLError(String),

    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

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

    /// Include resolved comments
    #[arg(short = 'a', long = "all")]
    pub include_resolved: bool,

    /// Show details for review number N
    #[arg(short = 'r', long = "review")]
    pub show_review: Option<usize>,

    /// Show all details
    #[arg(short = 'f', long = "full")]
    pub full_mode: bool,

    /// Expand <details> blocks
    #[arg(short = 'd', long = "open-details")]
    pub open_details: bool,
}

pub fn run(args: &CheckPrReviewArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let (owner, repo) = get_repo_owner_and_name(args.repo.as_deref())?;

    let pr_data = fetch_pr_data(&owner, &repo, args.pr_number, args.include_resolved)?;

    if pr_data.reviews.is_empty() && pr_data.threads.is_empty() {
        println!("No review comments found.");
        return Ok(());
    }

    let options = format::FormatOptions {
        open_details: args.open_details,
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

    // Get from current repo using gh CLI
    let output = std::process::Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "-q",
            ".nameWithOwner",
        ])
        .output()
        .map_err(|e| CheckPrReviewError::RepoInfoError(e.to_string()))?;

    if !output.status.success() {
        return Err(CheckPrReviewError::RepoInfoError(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    let name_with_owner = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let parts: Vec<&str> = name_with_owner.split('/').collect();
    if parts.len() == 2 {
        Ok((parts[0].to_string(), parts[1].to_string()))
    } else {
        Err(CheckPrReviewError::RepoInfoError(format!(
            "Unexpected repo format: {name_with_owner}"
        )))
    }
}
