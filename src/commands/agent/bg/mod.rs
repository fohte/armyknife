mod output;
mod run;
mod worker;

use anyhow::Result;
use clap::Subcommand;

pub use run::RunArgs;
pub use worker::RunDetachedArgs;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum BgCommands {
    /// Run a command in the background and notify this session when it finishes
    Run(RunArgs),

    /// Internal: wait for a background command and report its result
    #[command(name = "run-detached", hide = true)]
    RunDetached(RunDetachedArgs),
}

pub fn run(command: &BgCommands) -> Result<()> {
    match command {
        BgCommands::Run(args) => run::run(args),
        BgCommands::RunDetached(args) => worker::run(args),
    }
}
