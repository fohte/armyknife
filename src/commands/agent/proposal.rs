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
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct AddArgs {}

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
    }
}

fn add() -> Result<()> {
    let session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    let sessions_dir = store::sessions_dir()?;
    let stdin = std::io::stdin();
    let seq = record_proposal(&sessions_dir, &session_id, stdin.lock())?;
    println!("Recorded proposal {seq}");
    Ok(())
}

fn record_proposal(sessions_dir: &Path, session_id: &str, mut input: impl Read) -> Result<i64> {
    let mut raw = String::new();
    input
        .read_to_string(&mut raw)
        .context("reading proposal JSON from stdin")?;
    let body: ProposalBody = serde_json::from_str(&raw).context("parsing proposal JSON")?;
    validate_body(&body)?;
    let body_json = serde_json::to_string(&body).context("serializing validated proposal")?;
    store::add_agent_proposal_in(sessions_dir, session_id, &body_json)
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

    use rstest::{fixture, rstest};
    use serde_json::Value;
    use tempfile::TempDir;

    use super::record_proposal;

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().expect("temporary sessions directory should be created")
    }

    fn stored_proposal_bodies(sessions_dir: &Path) -> Vec<String> {
        crate::commands::agent::store::session_store_revision_from(sessions_dir)
            .expect("session store should initialize");
        let connection = rusqlite::Connection::open(sessions_dir.join("sessions.db"))
            .expect("session database should open");
        let mut statement = connection
            .prepare("SELECT body FROM agent_proposal WHERE session_id = 'session-a' ORDER BY seq")
            .expect("proposal query should prepare");
        statement
            .query_map([], |row| row.get(0))
            .expect("proposals should query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("proposal bodies should load")
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
        let bodies = stored_proposal_bodies(sessions_dir.path())
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
        let proposals = stored_proposal_bodies(sessions_dir.path());
        let error = result.err().map(|error| format!("{error:#}"));

        assert_eq!(
            (error, proposals),
            (Some(expected_error.to_string()), Vec::new())
        );
    }
}
