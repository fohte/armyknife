mod cli;
mod update;

use clap::Parser;
use cli::{Cli, Commands};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Cli { command } = Cli::parse();

    if command != Commands::Update {
        // Avoid running the updater twice when `a update` was requested.
        update::auto_update();
    }

    match command {
        Commands::Update => update::do_update()?,
    }

    Ok(())
}
