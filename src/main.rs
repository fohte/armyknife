mod cli;
mod update;

use clap::{CommandFactory, Parser};
use cli::{Cli, Commands};

fn main() {
    // Start update check in background (non-blocking)
    let update_rx = update::spawn_update_check();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Update) => {
            if let Err(e) = update::do_update() {
                eprintln!("Error updating: {e}");
                std::process::exit(1);
            }
        }
        None => {
            // No subcommand - show help
            Cli::command().print_help().ok();
            println!();
            update::print_update_notification(update_rx);
        }
    }
}
