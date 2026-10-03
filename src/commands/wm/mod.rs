mod clean;
mod error;
pub(crate) use error::WmError;
pub(crate) mod git;
mod list;
pub(crate) mod worktree;

#[cfg(test)]
mod tests;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum WmCommands {
    /// List all worktrees
    #[command(visible_alias = "ls")]
    List(list::ListArgs),

    /// Delete all merged worktrees
    #[command(visible_alias = "c")]
    Clean(clean::CleanArgs),
}

impl WmCommands {
    pub async fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::List(args) => list::run(args),
            Self::Clean(args) => clean::run(args).await,
        }
    }
}
