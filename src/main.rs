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
        Commands::Ai(ai_cmd) => ai_cmd.run()?,
        Commands::Update => update::do_update()?,
    }

    Ok(())
}
