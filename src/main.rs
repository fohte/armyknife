mod ai;
mod cli;
mod human_in_the_loop;
#[cfg(test)]
mod testing;
mod update;
mod wm;

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
        Commands::Wm(wm_cmd) => wm_cmd.run()?,
        Commands::Update => update::do_update()?,
    }

    Ok(())
}
