pub mod commands;
pub mod format;
pub mod models;
pub mod storage;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum IssueAgentCommands {
    /// View issue and comments (read-only)
    View(commands::ViewArgs),

    /// Fetch issue and save locally
    Pull(commands::PullArgs),

    /// Discard local changes and fetch latest
    Refresh(commands::RefreshArgs),

    /// Push local changes to GitHub
    Push(commands::PushArgs),
}

impl IssueAgentCommands {
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::View(args) => commands::run_view(args).await,
            Self::Pull(args) => commands::run_pull(args).await,
            Self::Refresh(args) => commands::run_refresh(args).await,
            Self::Push(args) => commands::run_push(args).await,
        }
    }
}
