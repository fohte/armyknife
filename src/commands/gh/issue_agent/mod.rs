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

    /// Review a file before pushing (opens editor, requires submit: true)
    Review(commands::ReviewArgs),

    /// Internal: Complete the review process after the editor exits
    #[command(hide = true)]
    ReviewComplete(commands::ReviewCompleteArgs),

    /// Push local changes to GitHub (requires file approval via review)
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
            Self::Review(args) => commands::run_review(args),
            Self::ReviewComplete(args) => commands::run_review_complete(args),
            Self::Push(args) => commands::run_push(args).await,
            Self::Diff(args) => commands::run_diff(args).await,
            Self::Init(args) => commands::run_init(args).await,
        }
    }
}
