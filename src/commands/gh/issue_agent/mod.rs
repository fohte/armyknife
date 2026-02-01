pub mod commands;
pub mod format;
pub mod models;
pub mod storage;
#[cfg(test)]
pub mod testing;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq, Debug)]
pub enum IssueAgentCommands {
    /// View issue and comments (read-only)
    View(commands::ViewArgs),

    /// Fetch issue and save locally (use --force to overwrite local changes)
    Pull(commands::PullArgs),

    /// Push local changes to GitHub
    Push(commands::PushArgs),

    /// Show diff between local changes and remote
    Diff(commands::DiffArgs),

    /// Initialize boilerplate for new issue or comment
    Init(commands::InitArgs),
}

impl IssueAgentCommands {
    pub async fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::View(args) => commands::run_view(args).await,
            Self::Pull(args) => commands::run_pull(args).await,
            Self::Push(args) => commands::run_push(args).await,
            Self::Diff(args) => commands::run_diff(args).await,
            Self::Init(args) => commands::run_init(args),
        }
    }
}
