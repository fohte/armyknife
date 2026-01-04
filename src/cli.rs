use clap::{Parser, Subcommand};

use crate::ai::AiCommands;
use crate::gh::GhCommands;

#[derive(Parser)]
#[command(
    name = "Fohte's armyknife",
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

    /// Update to the latest version
    Update,
}
