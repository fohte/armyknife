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

    /// Submit the draft as a pull request
    Submit(submit::SubmitArgs),
}

impl PrDraftCommands {
    pub fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::New(args) => new::run(args),
            Self::Review(args) => review::run(args),
            Self::Submit(args) => submit::run(args),
        }
    }
}
