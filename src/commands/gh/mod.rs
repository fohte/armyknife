pub mod issue_agent;
mod pr_merge;
pub mod pr_review;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum GhCommands {
    /// [Deprecated: use pr-review check] Fetch PR review comments in a concise format
    #[command(hide = true)]
    CheckPrReview(pr_review::CheckArgs),

    /// Manage PR review threads
    PrReview {
        #[command(subcommand)]
        command: pr_review::PrReviewCommands,
    },

    /// Safely squash-merge a pull request
    PrMerge(pr_merge::PrMergeArgs),

    /// Manage GitHub Issues as local files
    IssueAgent {
        #[command(subcommand)]
        command: issue_agent::IssueAgentCommands,
    },
}

impl GhCommands {
    pub async fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::CheckPrReview(args) => {
                eprintln!(
                    "Warning: 'check-pr-review' is deprecated. Use 'pr-review check' instead."
                );
                let pr_review_cmd = pr_review::PrReviewCommands::Check(args.clone());
                pr_review_cmd.run().await
            }
            Self::PrReview { command } => command.run().await,
            Self::PrMerge(args) => pr_merge::run(args).await,
            Self::IssueAgent { command } => command.run().await,
        }
    }
}
