pub mod pr_draft;
pub mod review;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum AiCommands {
    /// Manage PR draft files
    #[command(subcommand)]
    PrDraft(pr_draft::PrDraftCommands),

    /// Request or wait for bot reviews on a PR
    #[command(subcommand)]
    Review(review::ReviewCommands),
}

impl AiCommands {
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::PrDraft(cmd) => cmd.run().await,
            Self::Review(cmd) => cmd.run().await,
        }
    }
}
