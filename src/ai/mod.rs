pub mod pr_draft;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum AiCommands {
    /// Manage PR draft files
    #[command(subcommand)]
    PrDraft(pr_draft::PrDraftCommands),
}

impl AiCommands {
    pub fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::PrDraft(cmd) => cmd.run(),
        }
    }
}
