use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use clap::{Args, Subcommand};
use serde::Deserialize;

use super::error::CcError;
use super::store;
use crate::shared::env_var::EnvVars;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum TodoCommands {
    /// Record one task the user needs to do
    #[command(
        long_about = r#"Record one task the user needs to do for the calling session.
Use a proposal when the user should choose from options.

Read a JSON object from stdin with a required title and optional detail:

{
  "title": "Check the layout on an iPhone",
  "detail": "Open the preview and confirm the header fits."
}"#
    )]
    Add(AddArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct AddArgs {}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TodoBody {
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) detail: Option<String>,
}

pub fn run(command: &TodoCommands) -> Result<()> {
    match command {
        TodoCommands::Add(_) => add(),
    }
}

fn add() -> Result<()> {
    let session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    let sessions_dir = store::sessions_dir()?;
    let stdin = std::io::stdin();
    let seq = record_todo(&sessions_dir, &session_id, stdin.lock())?;
    println!("Recorded todo {seq}");
    Ok(())
}

fn record_todo(sessions_dir: &Path, session_id: &str, mut input: impl Read) -> Result<i64> {
    let mut raw = String::new();
    input
        .read_to_string(&mut raw)
        .context("reading todo JSON from stdin")?;
    let body: TodoBody = serde_json::from_str(&raw).context("parsing todo JSON")?;
    ensure!(
        !body.title.trim().is_empty(),
        "todo title must not be empty"
    );
    store::add_agent_todo_in(
        sessions_dir,
        session_id,
        &body.title,
        body.detail.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::record_todo;

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().expect("temporary sessions directory should be created")
    }

    fn stored_todos(sessions_dir: &Path) -> Vec<(String, Option<String>)> {
        crate::commands::agent::store::session_store_revision_from(sessions_dir)
            .expect("session store should initialize");
        let connection = rusqlite::Connection::open(sessions_dir.join("sessions.db"))
            .expect("session database should open");
        let mut statement = connection
            .prepare(
                "SELECT title, detail FROM agent_todo WHERE session_id = 'session-a' ORDER BY seq",
            )
            .expect("todo query should prepare");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("todos should query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("todo bodies should load")
    }

    #[rstest]
    fn records_a_todo_with_optional_detail(sessions_dir: TempDir) {
        let seq = record_todo(
            sessions_dir.path(),
            "session-a",
            std::io::Cursor::new(r#"{"title":"Check the page","detail":"Use a phone."}"#),
        )
        .expect("valid todo should save");

        assert_eq!(
            (seq, stored_todos(sessions_dir.path())),
            (
                1,
                vec![(
                    "Check the page".to_string(),
                    Some("Use a phone.".to_string())
                )],
            ),
        );
    }

    #[rstest]
    #[case::missing_title(
        r#"{"detail":"optional"}"#,
        "parsing todo JSON: missing field `title` at line 1 column 21"
    )]
    #[case::empty_title(r#"{"title":" "}"#, "todo title must not be empty")]
    #[case::unknown_field(
        r#"{"title":"Title","extra":true}"#,
        "parsing todo JSON: unknown field `extra`, expected `title` or `detail` at line 1 column 24"
    )]
    fn rejects_invalid_todos_without_saving(
        sessions_dir: TempDir,
        #[case] input: &str,
        #[case] expected_error: &str,
    ) {
        let result = record_todo(
            sessions_dir.path(),
            "session-a",
            std::io::Cursor::new(input),
        );
        let error = result.err().map(|error| format!("{error:#}"));

        assert_eq!(
            (error, stored_todos(sessions_dir.path())),
            (Some(expected_error.to_string()), Vec::new())
        );
    }
}
