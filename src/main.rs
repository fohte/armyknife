mod cli;
mod update;

use clap::{CommandFactory, Parser};
use cli::{Cli, Commands};

fn main() {
    // Auto-update if a new version is available (checked once per 24 hours)
    update::auto_update();

    let cli = Cli::parse();

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
