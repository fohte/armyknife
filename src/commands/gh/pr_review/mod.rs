mod api;
mod changeset;
mod error;
mod format;
mod markdown;
mod models;
mod reply;
mod review;
mod storage;

use clap::{Args, Subcommand};

use crate::infra::git;

pub use api::fetch_pr_data;
pub type Result<T> = anyhow::Result<T>;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum PrReviewCommands {
    /// View PR review comments in a concise format
    Check(CheckArgs),

    /// Manage PR review thread replies
    Reply {
        #[command(subcommand)]
        command: ReplyCommands,
    },
}

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum ReplyCommands {
    /// Pull review threads from GitHub to a local Markdown file
    Pull(reply::ReplyPullArgs),

    /// Push local reply drafts and resolve actions to GitHub
    Push(reply::ReplyPushArgs),

    /// Pull threads and open in editor for review (auto-pushes on approve)
    Review(review::ReviewArgs),

    /// Internal: Complete the review process after the editor exits
    #[command(hide = true)]
    ReviewComplete(review::ReviewCompleteArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CheckArgs {
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

impl PrReviewCommands {
    pub async fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::Check(args) => run_check(args).await,
            Self::Reply { command } => command.run().await,
        }
    }
}

impl ReplyCommands {
    pub async fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::Pull(args) => reply::run_pull(args).await,
            Self::Push(args) => reply::run_push(args).await,
            Self::Review(args) => review::run_review(args),
            Self::ReviewComplete(args) => review::run_review_complete(args).await,
        }
    }
}

async fn run_check(args: &CheckArgs) -> Result<()> {
    let (owner, repo) = git::get_repo_owner_and_name(args.repo.as_deref())?;

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
