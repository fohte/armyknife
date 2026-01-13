pub mod pr_draft;
pub mod review_gemini;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum AiCommands {
    /// Manage PR draft files
    #[command(subcommand)]
    PrDraft(pr_draft::PrDraftCommands),

    /// Wait for Gemini Code Assist review on a PR
    ReviewGemini(review_gemini::ReviewGeminiArgs),
}

impl AiCommands {
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::PrDraft(cmd) => cmd.run().await,
            Self::ReviewGemini(args) => {
                review_gemini::run(args).await?;
                Ok(())
            }
        }
    }
}
