pub mod check_pr_review;
pub mod issue_agent;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum GhCommands {
    /// Fetch PR review comments in a concise format
    CheckPrReview(check_pr_review::CheckPrReviewArgs),

    /// Manage GitHub Issues as local files
    IssueAgent {
        #[command(subcommand)]
        command: issue_agent::IssueAgentCommands,
    },
}

impl GhCommands {
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::CheckPrReview(args) => {
                check_pr_review::run(args).await?;
                Ok(())
            }
            Self::IssueAgent { command } => command.run().await,
        }
    }
}
