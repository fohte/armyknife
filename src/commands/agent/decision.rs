use anyhow::{Result, ensure};
use clap::{Args, Subcommand};

use super::error::CcError;
use super::store;
use crate::shared::env_var::EnvVars;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum DecisionCommands {
    /// Record a provisional decision
    Add(AddArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct AddArgs {
    /// Short description of the decision
    #[arg(value_name = "TITLE")]
    pub title: String,

    /// Why this option was chosen
    #[arg(long, value_name = "WHY")]
    pub why: String,

    /// Another option considered (repeatable)
    #[arg(long, required = true, value_name = "ALT")]
    pub considered: Vec<String>,

    /// How easily this decision can be changed
    #[arg(long, value_name = "TEXT")]
    pub easy_to_change: String,
}

const SESSION_START_INSTRUCTION: &str = "When unsure about a choice that is cheap to reverse, do not stop to ask. Pick an option, record it with `a agent decision add <TITLE> --why <WHY> --considered <ALT> --easy-to-change <TEXT>`, and keep working. Choices that are hard to reverse still require stopping and asking the user.";

pub fn run(command: &DecisionCommands) -> Result<()> {
    match command {
        DecisionCommands::Add(args) => add(args),
    }
}

fn add(args: &AddArgs) -> Result<()> {
    validate(args)?;
    let session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    let seq = store::insert_agent_decision(
        &store::sessions_dir()?,
        &session_id,
        &args.title,
        &args.why,
        &args.considered,
        &args.easy_to_change,
    )?;
    println!("Recorded decision {seq}");
    Ok(())
}

fn validate(args: &AddArgs) -> Result<()> {
    ensure!(!args.title.is_empty(), "decision title must not be empty");
    ensure!(!args.why.is_empty(), "decision why must not be empty");
    ensure!(
        !args.easy_to_change.is_empty(),
        "decision easy-to-change must not be empty"
    );
    ensure!(
        args.considered.iter().all(|option| !option.is_empty()),
        "decision considered options must not be empty"
    );
    Ok(())
}

pub(crate) fn session_start_instruction() -> &'static str {
    SESSION_START_INSTRUCTION
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use rstest::rstest;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: super::super::AgentCommands,
    }

    #[test]
    fn add_requires_at_least_one_considered_option() {
        let parsed = TestCli::try_parse_from([
            "a",
            "decision",
            "add",
            "sample title",
            "--why",
            "sample reason",
            "--easy-to-change",
            "sample change",
        ]);

        assert_eq!(
            parsed.err().map(|error| error.kind()),
            Some(clap::error::ErrorKind::MissingRequiredArgument),
        );
    }

    #[rstest]
    #[case::title(
        "",
        "reason",
        "alternative",
        "change",
        "decision title must not be empty"
    )]
    #[case::why("title", "", "alternative", "change", "decision why must not be empty")]
    #[case::considered(
        "title",
        "reason",
        "",
        "change",
        "decision considered options must not be empty"
    )]
    #[case::easy_to_change(
        "title",
        "reason",
        "alternative",
        "",
        "decision easy-to-change must not be empty"
    )]
    fn add_rejects_empty_fields(
        #[case] title: &str,
        #[case] why: &str,
        #[case] considered: &str,
        #[case] easy_to_change: &str,
        #[case] expected_error: &str,
    ) {
        let args = AddArgs {
            title: title.to_string(),
            why: why.to_string(),
            considered: vec![considered.to_string()],
            easy_to_change: easy_to_change.to_string(),
        };

        assert_eq!(
            validate(&args).map_err(|error| error.to_string()),
            Err(expected_error.to_string()),
        );
    }

    #[test]
    fn add_collects_repeated_considered_options() {
        let parsed = TestCli::try_parse_from([
            "a",
            "decision",
            "add",
            "sample title",
            "--why",
            "sample reason",
            "--considered",
            "first option",
            "--considered",
            "second option",
            "--easy-to-change",
            "sample change",
        ]);

        let considered = parsed.ok().map(|cli| match cli.command {
            super::super::AgentCommands::Decision(DecisionCommands::Add(args)) => args.considered,
            _ => Vec::new(),
        });

        assert_eq!(
            considered,
            Some(vec![
                "first option".to_string(),
                "second option".to_string()
            ]),
        );
    }
}
