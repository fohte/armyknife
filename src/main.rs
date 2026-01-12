mod ai;
mod cli;
mod gh;
mod git;
mod human_in_the_loop;
mod update;

use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Cli { command } = Cli::parse();

    if !matches!(command, Commands::Update) {
        // Avoid running the updater twice when `a update` was requested.
        update::auto_update();
    }

    match command {
        Commands::Ai(ai_cmd) => ai_cmd.run()?,
        Commands::Gh(gh_cmd) => gh_cmd.run()?,
        Commands::Update => update::do_update()?,
    }

    Ok(())
}
