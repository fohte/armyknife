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
struct ProposalBody {
    title: String,
    summary: Option<String>,
    options: Vec<ProposalOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalOption {
    label: String,
    description: Option<String>,
    #[serde(default)]
    pros: Vec<String>,
    #[serde(default)]
    cons: Vec<String>,
    #[serde(default)]
    recommended: bool,
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
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("reading proposal JSON from stdin")?;
    let seq = add_from_json(&sessions_dir, &session_id, &input)?;
    println!("Recorded proposal {seq}");
    Ok(())
}

fn add_from_json(sessions_dir: &Path, session_id: &str, input: &str) -> Result<i64> {
    let body: ProposalBody = serde_json::from_str(input).context("parsing proposal JSON")?;
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
    use rstest::{fixture, rstest};
    use serde_json::Value;
    use tempfile::TempDir;

    use super::add_from_json;
    use crate::commands::agent::store::agent_proposals_from;

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().expect("temporary sessions directory should be created")
    }

    #[rstest]
    fn records_a_validated_proposal_with_normalized_optional_fields(sessions_dir: TempDir) {
        let input = r#"{"title":"Pick a format","options":[{"label":"Format A","recommended":true},{"label":"Format B"}]}"#;

        let seq = add_from_json(sessions_dir.path(), "session-a", input)
            .expect("valid proposal should save");
        let proposals =
            agent_proposals_from(sessions_dir.path(), "session-a").expect("proposal should load");
        let bodies = proposals
            .iter()
            .map(|proposal| serde_json::from_str::<Value>(&proposal.body))
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
        let result = add_from_json(sessions_dir.path(), "session-a", input);
        let proposals = agent_proposals_from(sessions_dir.path(), "session-a")
            .expect("proposal list should load");
        let error = result.err().map(|error| format!("{error:#}"));

        assert_eq!(
            (error, proposals),
            (Some(expected_error.to_string()), Vec::new())
        );
    }
}
