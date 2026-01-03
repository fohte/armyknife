mod cli;
mod update;

use clap::{CommandFactory, Parser};
use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();

    if !matches!(cli.command, Some(Commands::Update)) {
        // Avoid running the updater twice when `a update` was requested.
        update::auto_update();
    }

    match cli.command {
        Some(Commands::Update) => {
            if let Err(e) = update::do_update() {
                eprintln!("Error updating: {e}");
                std::process::exit(1);
            }
        }
        None => {
            Cli::command().print_help().ok();
            println!();
        }
    }
}
