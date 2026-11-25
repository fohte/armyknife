use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};

use super::error::CcError;
use super::store;
use crate::shared::env_var::EnvVars;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum ProposalCommands {
    /// Record a proposal from JSON read from stdin
    #[command(
        long_about = r#"Record a proposal for the calling session. The JSON must have this shape:

{
  "title": "Choose a storage strategy",
  "summary": "Optional context for the choice",
  "options": [
    {
      "label": "Store it on disk",
      "description": "Write data after each update",
      "pros": ["Survives restarts"],
      "cons": ["Needs cleanup"],
      "recommended": true
    },
    {
      "label": "Keep it in memory"
    }
  ]
}

The title and each option label are required. There must be 2 to 9 options,
and at most one option may be recommended. Optional fields are summary,
description, pros, cons, and recommended."#
    )]
    Add(AddArgs),
    /// Replace one of this session's pending proposals from JSON read from stdin
    Edit(EditArgs),
    /// Withdraw one of this session's pending proposals
    Withdraw(WithdrawArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct AddArgs {}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct EditArgs {
    /// Sequence number printed when the proposal was recorded
    seq: i64,
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WithdrawArgs {
    /// Sequence number printed when the proposal was recorded
    seq: i64,
    /// Optional reason for withdrawing the proposal
    #[arg(long)]
    comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProposalBody {
    pub(crate) title: String,
    pub(crate) summary: Option<String>,
    pub(crate) options: Vec<ProposalOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProposalOption {
    pub(crate) label: String,
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) pros: Vec<String>,
    #[serde(default)]
    pub(crate) cons: Vec<String>,
    #[serde(default)]
    pub(crate) recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProposalAnswer {
    pub(crate) choice: Option<usize>,
    pub(crate) comment: Option<String>,
}

pub fn run(command: &ProposalCommands) -> Result<()> {
    match command {
        ProposalCommands::Add(_) => add(),
        ProposalCommands::Edit(args) => edit(args),
        ProposalCommands::Withdraw(args) => withdraw(args),
    }
}

fn edit(args: &EditArgs) -> Result<()> {
    let session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    let sessions_dir = store::sessions_dir()?;
    let stdin = std::io::stdin();
    edit_proposal(&sessions_dir, &session_id, args.seq, stdin.lock())?;
    println!("Edited proposal {}", args.seq);
    Ok(())
}

fn edit_proposal(sessions_dir: &Path, session_id: &str, seq: i64, input: impl Read) -> Result<()> {
    let body = read_proposal_body(input)?;
    let body_json = serde_json::to_string(&body).context("serializing validated proposal")?;
    let _proposal_lock = store::lock_agent_proposal_resolution_from(sessions_dir, session_id, seq)?;
    ensure!(
        store::edit_agent_proposal_from(sessions_dir, session_id, seq, &body_json)?,
        "proposal is already resolved or does not exist"
    );
    Ok(())
}

fn withdraw(args: &WithdrawArgs) -> Result<()> {
    let session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    let sessions_dir = store::sessions_dir()?;
    withdraw_proposal(
        &sessions_dir,
        &session_id,
        args.seq,
        args.comment.as_deref(),
    )?;
    println!("Withdrew proposal {}", args.seq);
    Ok(())
}

fn withdraw_proposal(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    comment: Option<&str>,
) -> Result<()> {
    let comment = comment.map(str::trim).filter(|comment| !comment.is_empty());
    let _proposal_lock = store::lock_agent_proposal_resolution_from(sessions_dir, session_id, seq)?;
    ensure!(
        store::resolve_agent_proposal_from(sessions_dir, session_id, seq, comment)?,
        "proposal is already resolved or does not exist"
    );
    Ok(())
}

fn add() -> Result<()> {
    add_from_input(std::io::stdin().lock())
}

fn add_from_input(input: impl Read) -> Result<()> {
    let session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    let sessions_dir = store::sessions_dir()?;
    let seq = record_proposal(&sessions_dir, &session_id, input)?;
    println!("Recorded proposal {seq}");
    Ok(())
}

fn record_proposal(sessions_dir: &Path, session_id: &str, mut input: impl Read) -> Result<i64> {
    let body = read_proposal_body(&mut input)?;
    let body_json = serde_json::to_string(&body).context("serializing validated proposal")?;
    store::add_agent_proposal_in(sessions_dir, session_id, &body_json)
}

fn read_proposal_body(mut input: impl Read) -> Result<ProposalBody> {
    let mut raw = String::new();
    input
        .read_to_string(&mut raw)
        .context("reading proposal JSON from stdin")?;
    let body: ProposalBody = serde_json::from_str(&raw).context("parsing proposal JSON")?;
    validate_body(&body)?;
    Ok(body)
}

fn validate_body(body: &ProposalBody) -> Result<()> {
    ensure!(
        !body.title.trim().is_empty(),
        "proposal title must not be empty"
    );
    ensure!(
        (2..=9).contains(&body.options.len()),
        "proposal must contain between 2 and 9 options"
    );
    ensure!(
        body.options
            .iter()
            .all(|option| !option.label.trim().is_empty()),
        "proposal option labels must not be empty"
    );
    ensure!(
        body.options
            .iter()
            .filter(|option| option.recommended)
            .count()
            <= 1,
        "proposal may have at most one recommended option"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::Parser;
    use rstest::{fixture, rstest};
    use serde_json::Value;
    use tempfile::TempDir;

    use super::{
        ProposalCommands, add_from_input, edit_proposal, record_proposal, withdraw_proposal,
    };
    use crate::commands::agent::store::{self, ProposalResolution};

    #[derive(Parser)]
    struct ProposalCli {
        #[command(subcommand)]
        command: ProposalCommands,
    }

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().expect("temporary sessions directory should be created")
    }

    fn stored_proposal_bodies(sessions_dir: &Path, session_id: &str) -> Vec<String> {
        crate::commands::agent::store::session_store_revision_from(sessions_dir)
            .expect("session store should initialize");
        let connection = rusqlite::Connection::open(sessions_dir.join("sessions.db"))
            .expect("session database should open");
        let mut statement = connection
            .prepare("SELECT body FROM agent_proposal WHERE session_id = ?1 ORDER BY seq")
            .expect("proposal query should prepare");
        statement
            .query_map([session_id], |row| row.get(0))
            .expect("proposals should query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("proposal bodies should load")
    }

    #[rstest]
    fn withdraws_a_pending_proposal_with_a_trimmed_comment(sessions_dir: TempDir) {
        let seq = store::add_agent_proposal_in(sessions_dir.path(), "session-a", "{}")
            .expect("proposal should save");
        withdraw_proposal(
            sessions_dir.path(),
            "session-a",
            seq,
            Some("  The decision changed.  "),
        )
        .expect("proposal should be withdrawn");

        let actual = store::agent_proposals_from(sessions_dir.path(), "session-a")
            .expect("proposal should load")
            .into_iter()
            .map(|proposal| (proposal.resolution, proposal.comment))
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            vec![(
                Some(ProposalResolution::Withdrawn),
                Some("The decision changed.".to_string()),
            )],
        );
    }

    #[rstest]
    fn rejects_withdrawing_another_sessions_proposal(sessions_dir: TempDir) {
        let seq = store::add_agent_proposal_in(sessions_dir.path(), "session-b", "{}")
            .expect("other session proposal should save");
        let error = withdraw_proposal(sessions_dir.path(), "session-a", seq, None)
            .expect_err("another session proposal should not be addressable");

        assert_eq!(
            error.to_string(),
            "proposal is already resolved or does not exist"
        );
    }

    #[rstest]
    fn rejects_withdrawing_an_answered_or_withdrawn_proposal(sessions_dir: TempDir) {
        let answered = store::add_agent_proposal_in(sessions_dir.path(), "session-a", "{}")
            .expect("proposal should save");
        store::write_agent_proposal_answer_at_from(
            sessions_dir.path(),
            "session-a",
            answered,
            "{}",
            chrono::Utc::now(),
        )
        .expect("answer should save");
        let withdrawn = store::add_agent_proposal_in(sessions_dir.path(), "session-a", "{}")
            .expect("proposal should save");
        withdraw_proposal(sessions_dir.path(), "session-a", withdrawn, None)
            .expect("proposal should be withdrawn");

        let results = [answered, withdrawn].map(|seq| {
            withdraw_proposal(sessions_dir.path(), "session-a", seq, None)
                .err()
                .map(|error| error.to_string())
        });
        assert_eq!(
            results,
            [
                Some("proposal is already resolved or does not exist".to_string()),
                Some("proposal is already resolved or does not exist".to_string()),
            ],
        );
    }

    #[rstest]
    fn replaces_a_pending_proposal_without_changing_its_identity_or_position(
        sessions_dir: TempDir,
    ) {
        let seq = store::add_agent_proposal_in(sessions_dir.path(), "session-a", "original")
            .expect("proposal should save");
        let before = store::agent_proposals_from(sessions_dir.path(), "session-a")
            .expect("proposal should load");
        let before_revision =
            store::session_store_revision_from(sessions_dir.path()).expect("revision should load");
        edit_proposal(
            sessions_dir.path(),
            "session-a",
            seq,
            std::io::Cursor::new(
                r#"{"title":"Choose a format","summary":"Updated context","options":[{"label":"Compact","pros":["Small"],"recommended":true},{"label":"Readable","cons":["Longer"]}]}"#,
            ),
        )
        .expect("pending proposal should be editable");
        let after = store::agent_proposals_from(sessions_dir.path(), "session-a")
            .expect("updated proposal should load");
        let after_revision =
            store::session_store_revision_from(sessions_dir.path()).expect("revision should load");

        assert_eq!(
            (
                before[0].seq,
                before[0].created_at,
                after
                    .into_iter()
                    .map(|proposal| (
                        proposal.seq,
                        proposal.body,
                        proposal.created_at,
                        proposal.answer,
                        proposal.resolved_at,
                        proposal.resolution,
                        proposal.comment,
                    ))
                    .collect::<Vec<_>>(),
                after_revision,
            ),
            (
                seq,
                before[0].created_at,
                vec![(
                    seq,
                    r#"{"title":"Choose a format","summary":"Updated context","options":[{"label":"Compact","description":null,"pros":["Small"],"cons":[],"recommended":true},{"label":"Readable","description":null,"pros":[],"cons":["Longer"],"recommended":false}]}"#.to_string(),
                    before[0].created_at,
                    None,
                    None,
                    None,
                    None,
                )],
                before_revision + 1,
            ),
        );
    }

    #[rstest]
    #[case::missing("missing")]
    #[case::another_session("another_session")]
    #[case::answered("answered")]
    #[case::withdrawn("withdrawn")]
    fn refuses_to_edit_a_proposal_that_is_not_pending(sessions_dir: TempDir, #[case] target: &str) {
        let seq = if target == "another_session" {
            store::add_agent_proposal_in(sessions_dir.path(), "session-b", "original")
                .expect("other session proposal should save")
        } else if target == "missing" {
            1
        } else {
            let seq = store::add_agent_proposal_in(sessions_dir.path(), "session-a", "original")
                .expect("proposal should save");
            match target {
                "answered" => {
                    store::write_agent_proposal_answer_at_from(
                        sessions_dir.path(),
                        "session-a",
                        seq,
                        "answer",
                        chrono::Utc::now(),
                    )
                    .expect("answer should save");
                }
                "withdrawn" => {
                    store::resolve_agent_proposal_from(sessions_dir.path(), "session-a", seq, None)
                        .expect("proposal should be withdrawn");
                }
                _ => unreachable!(),
            }
            seq
        };
        let before = store::agent_proposals_from(sessions_dir.path(), "session-a")
            .expect("proposals should load");
        let before_revision =
            store::session_store_revision_from(sessions_dir.path()).expect("revision should load");
        let error = edit_proposal(
            sessions_dir.path(),
            "session-a",
            seq,
            std::io::Cursor::new(
                r#"{"title":"Replacement","options":[{"label":"A"},{"label":"B"}]}"#,
            ),
        )
        .err()
        .map(|error| error.to_string());
        let after = store::agent_proposals_from(sessions_dir.path(), "session-a")
            .expect("proposals should load");
        let after_revision =
            store::session_store_revision_from(sessions_dir.path()).expect("revision should load");

        assert_eq!(
            (
                error,
                before.clone(),
                after,
                before_revision,
                after_revision
            ),
            (
                Some("proposal is already resolved or does not exist".to_string()),
                before.clone(),
                before,
                before_revision,
                before_revision,
            ),
        );
    }

    #[rstest]
    fn rejects_answering_a_withdrawn_proposal(sessions_dir: TempDir) {
        let seq = store::add_agent_proposal_in(sessions_dir.path(), "session-a", "{}")
            .expect("proposal should save");
        withdraw_proposal(sessions_dir.path(), "session-a", seq, None)
            .expect("proposal should be withdrawn");

        let answer_after_withdrawal = store::write_agent_proposal_answer_at_from(
            sessions_dir.path(),
            "session-a",
            seq,
            "{}",
            chrono::Utc::now(),
        )
        .expect("withdrawn proposal answer should be rejected");
        assert!(!answer_after_withdrawal);
    }

    #[rstest]
    fn parses_the_withdraw_command_and_optional_comment() {
        let cli = ProposalCli::try_parse_from([
            "proposal",
            "withdraw",
            "3",
            "--comment",
            "No longer needed.",
        ])
        .expect("withdraw arguments should parse");
        let actual = match cli.command {
            ProposalCommands::Withdraw(args) => (args.seq, args.comment),
            ProposalCommands::Add(_) | ProposalCommands::Edit(_) => (0, None),
        };
        assert_eq!(actual, (3, Some("No longer needed.".to_string())));
    }

    #[rstest]
    fn parses_the_edit_command_sequence() {
        let cli = ProposalCli::try_parse_from(["proposal", "edit", "3"])
            .expect("edit arguments should parse");
        let actual = match cli.command {
            ProposalCommands::Edit(args) => args.seq,
            ProposalCommands::Add(_) | ProposalCommands::Withdraw(_) => 0,
        };

        assert_eq!(actual, 3);
    }

    #[rstest]
    fn records_a_validated_proposal_with_normalized_optional_fields(sessions_dir: TempDir) {
        let input = r#"{"title":"Pick a format","options":[{"label":"Format A","recommended":true},{"label":"Format B"}]}"#;

        let seq = record_proposal(
            sessions_dir.path(),
            "session-a",
            std::io::Cursor::new(input),
        )
        .expect("valid proposal should save");
        let bodies = stored_proposal_bodies(sessions_dir.path(), "session-a")
            .iter()
            .map(|body| serde_json::from_str::<Value>(body))
            .collect::<serde_json::Result<Vec<_>>>()
            .expect("stored proposals should remain JSON");

        assert_eq!(
            (seq, bodies),
            (
                1,
                vec![serde_json::json!({
                    "title": "Pick a format",
                    "summary": null,
                    "options": [
                        {
                            "label": "Format A",
                            "description": null,
                            "pros": [],
                            "cons": [],
                            "recommended": true,
                        },
                        {
                            "label": "Format B",
                            "description": null,
                            "pros": [],
                            "cons": [],
                            "recommended": false,
                        },
                    ],
                })],
            ),
        );
    }

    #[rstest]
    #[case::missing_title(
        r#"{"options":[{"label":"A"},{"label":"B"}]}"#,
        "parsing proposal JSON: missing field `title` at line 1 column 41"
    )]
    #[case::empty_title(
        r#"{"title":" ","options":[{"label":"A"},{"label":"B"}]}"#,
        "proposal title must not be empty"
    )]
    #[case::one_option(
        r#"{"title":"Title","options":[{"label":"A"}]}"#,
        "proposal must contain between 2 and 9 options"
    )]
    #[case::ten_options(
        r#"{"title":"Title","options":[{"label":"A"},{"label":"B"},{"label":"C"},{"label":"D"},{"label":"E"},{"label":"F"},{"label":"G"},{"label":"H"},{"label":"I"},{"label":"J"}]}"#,
        "proposal must contain between 2 and 9 options"
    )]
    #[case::empty_label(
        r#"{"title":"Title","options":[{"label":" "},{"label":"B"}]}"#,
        "proposal option labels must not be empty"
    )]
    #[case::multiple_recommendations(r#"{"title":"Title","options":[{"label":"A","recommended":true},{"label":"B","recommended":true}]}"#, "proposal may have at most one recommended option")]
    #[case::unknown_field(
        r#"{"title":"Title","options":[{"label":"A"},{"label":"B"}],"extra":true}"#,
        "parsing proposal JSON: unknown field `extra`, expected one of `title`, `summary`, `options` at line 1 column 64"
    )]
    fn rejects_invalid_proposals_without_saving(
        sessions_dir: TempDir,
        #[case] input: &str,
        #[case] expected_error: &str,
    ) {
        let result = record_proposal(
            sessions_dir.path(),
            "session-a",
            std::io::Cursor::new(input),
        );
        let proposals = stored_proposal_bodies(sessions_dir.path(), "session-a");
        let error = result.err().map(|error| format!("{error:#}"));

        assert_eq!(
            (error, proposals),
            (Some(expected_error.to_string()), Vec::new())
        );
    }

    #[rstest]
    fn add_records_a_proposal_for_the_claude_code_session_id() {
        let cache_home = tempfile::tempdir().expect("temporary cache home should be created");
        let sessions_dir = cache_home.path().join("armyknife/cc/sessions");
        let cache_home_path = cache_home
            .path()
            .to_str()
            .expect("cache path should be valid UTF-8");
        let input =
            r#"{"title":"Pick a format","options":[{"label":"Format A"},{"label":"Format B"}]}"#;

        temp_env::with_vars(
            [
                ("XDG_CACHE_HOME", Some(cache_home_path)),
                ("ARMYKNIFE_SESSION_ID", None::<&str>),
                ("CLAUDE_CODE_SESSION_ID", Some("claude-session")),
                ("CODEX_SESSION_ID", None::<&str>),
            ],
            || {
                add_from_input(std::io::Cursor::new(input))
                    .expect("proposal should be saved for the Claude Code session");
            },
        );

        let actual = stored_proposal_bodies(&sessions_dir, "claude-session");
        assert_eq!(
            actual,
            vec![r#"{"title":"Pick a format","summary":null,"options":[{"label":"Format A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Format B","description":null,"pros":[],"cons":[],"recommended":false}]}"#.to_string()],
        );
    }
}
