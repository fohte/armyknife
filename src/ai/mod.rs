pub mod pr_draft;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum AiCommands {
    /// Manage PR draft files
    #[command(subcommand)]
    PrDraft(pr_draft::PrDraftCommands),
}
