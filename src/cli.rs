use clap::{Parser, Subcommand};
use clap_complete::Shell;

use crate::ai::AiCommands;
use crate::gh::GhCommands;
use crate::name_branch::NameBranchArgs;
use crate::wm::WmCommands;

#[derive(Parser)]
#[command(
    name = "armyknife",
    bin_name = "a",
    version,
    about,
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum Commands {
    /// AI-related tools
    #[command(subcommand)]
    Ai(AiCommands),

    /// GitHub-related tools
    #[command(subcommand)]
    Gh(GhCommands),

    /// Generate a branch name from a description using AI
    NameBranch(NameBranchArgs),

    /// Git worktree manager
    #[command(subcommand)]
    Wm(WmCommands),

    /// Update to the latest version
    Update,

    /// Generate shell completion scripts
    Completions {
        /// Target shell
        #[arg(value_enum)]
        shell: Shell,
    },
}
