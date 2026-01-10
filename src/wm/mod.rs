mod clean;
pub mod common;
mod delete;
mod list;
mod new;

#[cfg(test)]
mod tests;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum WmCommands {
    /// List all worktrees
    List(list::ListArgs),

    /// Create a new Git worktree for a branch
    New(new::NewArgs),

    /// Delete a Git worktree and its branch
    Delete(delete::DeleteArgs),

    /// Delete all merged worktrees
    Clean(clean::CleanArgs),
}

impl WmCommands {
    pub fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::List(args) => list::run(args),
            Self::New(args) => new::run(args),
            Self::Delete(args) => delete::run(args),
            Self::Clean(args) => clean::run(args),
        }
    }
}
