//! Review commands for bot reviewers.

mod common;
mod error;
pub mod request;
mod reviewer;
pub mod wait;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum ReviewCommands {
    /// Request a review from a bot reviewer and wait for completion
    Request(request::RequestArgs),

    /// Wait for an existing review to complete (does not trigger new review)
    Wait(wait::WaitArgs),
}

impl ReviewCommands {
    pub async fn run(&self) -> std::result::Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::Request(args) => {
                request::run(args).await?;
                Ok(())
            }
            Self::Wait(args) => {
                wait::run(args).await?;
                Ok(())
            }
        }
    }
}
