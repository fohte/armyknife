mod ai;
mod cli;
mod update;

use clap::Parser;
use cli::{Cli, Commands};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Cli { command } = Cli::parse();

    if !matches!(command, Commands::Update) {
        // Avoid running the updater twice when `a update` was requested.
        update::auto_update();
    }

    match command {
        Commands::Ai(ai_cmd) => match ai_cmd {
            ai::AiCommands::PrDraft(pr_draft_cmd) => pr_draft_cmd.run()?,
        },
        Commands::Update => update::do_update()?,
    }

    Ok(())
}
