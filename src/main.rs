use anyhow::Result;
use armyknife::cli::{Cli, Commands};
use armyknife::commands::agent::{AgentCommands, BgCommands};
use armyknife::shared;
use armyknife::shared::update;
use clap::{CommandFactory, Parser};
use clap_complete::aot::generate;

#[tokio::main]
async fn main() {
    shared::log::init();
    match run().await {
        Ok(ExitMode::Immediate) => std::process::exit(0),
        Ok(ExitMode::ImmediateWithCode(code)) => std::process::exit(code),
        Ok(ExitMode::RuntimeShutdown) => {}
        Err(e) => {
            eprintln!("Error: {e:?}");
            std::process::exit(1);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ExitMode {
    RuntimeShutdown,
    Immediate,
    ImmediateWithCode(i32),
}

fn exit_mode(command: &Commands) -> ExitMode {
    match command {
        Commands::Agent(AgentCommands::Watch(_)) => ExitMode::Immediate,
        _ => ExitMode::RuntimeShutdown,
    }
}

async fn run() -> Result<ExitMode> {
    let Cli { command } = Cli::parse();
    let exit_mode = exit_mode(&command);

    if !matches!(
        &command,
        Commands::Update
            | Commands::Completions { .. }
            | Commands::Doctor(_)
            | Commands::Agent(AgentCommands::ArchiveDetached(_))
            | Commands::Agent(AgentCommands::Bg(BgCommands::ReminderHook(_)))
    ) {
        // Avoid running the updater twice when `a update` was requested,
        // and skip for completions to keep output clean.
        // Skip for `doctor` so the diagnostic snapshot reflects the current
        // binary rather than the post-update one.
        update::auto_update().await;
    }

    match command {
        Commands::Agent(agent_cmd) => {
            if let Some(code) = agent_cmd.run().await? {
                return Ok(ExitMode::ImmediateWithCode(code));
            }
        }
        Commands::Ai(ai_cmd) => ai_cmd.run().await?,
        Commands::Config(config_cmd) => config_cmd.run().await?,
        Commands::Doctor(args) => armyknife::commands::doctor::run(&args)?,
        Commands::Gh(gh_cmd) => gh_cmd.run().await?,
        Commands::NameBranch(args) => args.run()?,
        Commands::Update => update::do_update()?,
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "a", &mut std::io::stdout());
        }
    }

    Ok(exit_mode)
}

#[cfg(test)]
mod tests {
    use super::{Commands, ExitMode, exit_mode};
    use armyknife::commands::agent::{AgentCommands, WatchArgs};
    use rstest::rstest;

    #[rstest]
    #[case::agent_watch(
        Commands::Agent(AgentCommands::Watch(WatchArgs {})),
        ExitMode::Immediate
    )]
    #[case::other_command(Commands::Update, ExitMode::RuntimeShutdown)]
    fn test_exit_mode(#[case] command: Commands, #[case] expected: ExitMode) {
        assert_eq!(exit_mode(&command), expected);
    }
}
