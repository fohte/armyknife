use clap::{Parser, Subcommand};
use clap_complete::Shell;

use crate::commands::ai::AiCommands;
use crate::commands::cc::CcCommands;
use crate::commands::config::ConfigCommands;
use crate::commands::gh::GhCommands;
use crate::commands::name_branch::NameBranchArgs;
use crate::commands::wm::WmCommands;

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

    /// Claude Code session monitor
    #[command(subcommand)]
    Cc(CcCommands),

    /// GitHub-related tools
    #[command(subcommand)]
    Gh(GhCommands),

    /// Configuration management
    #[command(subcommand)]
    Config(ConfigCommands),

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
