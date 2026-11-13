use anyhow::Result;
use clap::{Args, Subcommand};

use super::error::CcError;
use super::session_title;
use crate::shared::env_var::EnvVars;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum TitleCommands {
    /// Set the calling session's title; pass an empty string to restore its automatic title
    Set(SetArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SetArgs {
    /// New title (empty restores the automatic title)
    pub title: String,
}

pub fn run(command: &TitleCommands) -> Result<()> {
    match command {
        TitleCommands::Set(args) => set(&args.title),
    }
}

fn set(title: &str) -> Result<()> {
    let session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    session_title::set_session_title(&session_id, title)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::commands::agent::AgentCommands;

    use super::TitleCommands;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: AgentCommands,
    }

    #[rstest::rstest]
    #[case::title("New session title", "New session title")]
    #[case::empty("", "")]
    fn parses_title_set(#[case] title: &str, #[case] expected: &str) {
        let parsed = TestCli::try_parse_from(["a", "title", "set", title])
            .expect("title set command should parse");
        let AgentCommands::Title(TitleCommands::Set(args)) = parsed.command else {
            panic!("title set should select the title command");
        };

        assert_eq!(args.title, expected);
    }
}
