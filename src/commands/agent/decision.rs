use std::io::{self, Write};
use std::path::Path;

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
    let sessions_dir = store::sessions_dir()?;
    let mut stdout = io::stdout().lock();
    add_in(args, &session_id, &sessions_dir, &mut stdout)
}

fn add_in(
    args: &AddArgs,
    session_id: &str,
    sessions_dir: &Path,
    output: &mut impl Write,
) -> Result<()> {
    let seq = store::insert_agent_decision(
        sessions_dir,
        session_id,
        &args.title,
        &args.why,
        &args.considered,
        &args.easy_to_change,
    )?;
    writeln!(output, "Recorded decision {seq}")?;
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
    use crate::commands::agent::types::{Engine, Session, SessionStatus};
    use clap::Parser;
    use indoc::indoc;
    use rstest::rstest;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: super::super::AgentCommands,
    }

    fn test_session(session_id: &str) -> Session {
        Session {
            session_id: session_id.to_string(),
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/workspace/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            launched_with_vibe: false,
            work_type_pinned: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }

    fn normalize_timestamps(mut decision: store::AgentDecision) -> (store::AgentDecision, bool) {
        let created_at_is_rfc3339 =
            chrono::DateTime::parse_from_rfc3339(&decision.created_at).is_ok();
        decision.created_at = "normalized timestamp".to_string();
        (decision, created_at_is_rfc3339)
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

    #[test]
    fn add_records_the_decision_and_prints_its_sequence() {
        let sessions_dir = TempDir::new().expect("temporary directory should be created");
        let session_id = "session-cobalt";
        store::save_session_to(sessions_dir.path(), &test_session(session_id))
            .expect("session should be stored");
        let args = AddArgs {
            title: "title".to_string(),
            why: "reason".to_string(),
            considered: vec![
                "first alternative".to_string(),
                "second alternative".to_string(),
            ],
            easy_to_change: "change is local".to_string(),
        };
        let mut output = Vec::new();

        add_in(&args, session_id, sessions_dir.path(), &mut output)
            .expect("decision should be recorded");
        let decisions = store::agent_decisions_from(sessions_dir.path(), session_id)
            .expect("decision should load");

        assert_eq!(
            (
                decisions
                    .into_iter()
                    .map(normalize_timestamps)
                    .collect::<Vec<_>>(),
                output,
            ),
            (
                vec![(
                    store::AgentDecision {
                        session_id: session_id.to_string(),
                        seq: 1,
                        title: "title".to_string(),
                        why: "reason".to_string(),
                        considered: vec![
                            "first alternative".to_string(),
                            "second alternative".to_string(),
                        ],
                        easy_to_change: "change is local".to_string(),
                        created_at: "normalized timestamp".to_string(),
                        review: None,
                        reviewed_at: None,
                    },
                    true
                )],
                indoc! {"
                    Recorded decision 1
                "}
                .as_bytes()
                .to_vec(),
            ),
        );
    }
}
