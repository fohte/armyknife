mod clean;
mod delete;
mod error;
mod git;
mod list;
mod new;
pub(crate) mod worktree;

#[cfg(test)]
mod tests;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum WmCommands {
    /// List all worktrees
    #[command(visible_alias = "ls")]
    List(list::ListArgs),

    /// Create a new Git worktree for a branch
    New(new::NewArgs),

    /// Delete a Git worktree and its branch
    #[command(visible_aliases = ["d", "rm"])]
    Delete(delete::DeleteArgs),

    /// Delete all merged worktrees
    #[command(visible_alias = "c")]
    Clean(clean::CleanArgs),
}

impl WmCommands {
    pub async fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::List(args) => list::run(args),
            Self::New(args) => new::run(args),
            Self::Delete(args) => delete::run(args).await,
            Self::Clean(args) => clean::run(args).await,
        }
    }
}
