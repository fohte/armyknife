use anyhow::Result;
use armyknife::cli::{Cli, Commands};
use armyknife::shared;
use armyknife::shared::update;
use clap::{CommandFactory, Parser};
use clap_complete::aot::generate;

#[tokio::main]
async fn main() {
    shared::log::init();
    if let Err(e) = run().await {
        eprintln!("Error: {e:?}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let Cli { command } = Cli::parse();

    if !matches!(
        command,
        Commands::Update | Commands::Completions { .. } | Commands::Doctor(_)
    ) {
        // Avoid running the updater twice when `a update` was requested,
        // and skip for completions to keep output clean.
        // Skip for `doctor` so the diagnostic snapshot reflects the current
        // binary rather than the post-update one.
        update::auto_update().await;
    }

    match command {
        Commands::Ai(ai_cmd) => ai_cmd.run().await?,
        Commands::Cc(cc_cmd) => cc_cmd.run().await?,
        Commands::Config(config_cmd) => config_cmd.run().await?,
        Commands::Doctor(args) => armyknife::commands::doctor::run(&args)?,
        Commands::Gh(gh_cmd) => gh_cmd.run().await?,
        Commands::NameBranch(args) => args.run()?,
        Commands::Wm(wm_cmd) => wm_cmd.run().await?,
        Commands::Update => update::do_update()?,
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "a", &mut std::io::stdout());
        }
    }

    Ok(())
}
