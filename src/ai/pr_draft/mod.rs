mod common;
mod new;
mod review;
mod submit;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum PrDraftCommands {
    /// Create a new PR body draft file
    New(new::NewArgs),

    /// Open the draft file in an editor for review
    Review(review::ReviewArgs),

    /// Internal: Complete the review process after Neovim exits
    #[command(hide = true)]
    ReviewComplete(review::ReviewCompleteArgs),

    /// Submit the draft as a pull request
    Submit(submit::SubmitArgs),
}

impl PrDraftCommands {
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::New(args) => new::run(args).await,
            Self::Review(args) => review::run(args),
            Self::ReviewComplete(args) => review::run_complete(args),
            Self::Submit(args) => submit::run(args).await,
        }
    }
}
