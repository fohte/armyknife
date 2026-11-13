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
    /// Withdraw one of this session's pending tasks
    Withdraw(WithdrawArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct AddArgs {}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WithdrawArgs {
    /// Sequence number printed when the todo was recorded
    seq: i64,
    /// Optional reason for withdrawing the todo
    #[arg(long)]
    comment: Option<String>,
}

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
        TodoCommands::Withdraw(args) => withdraw(args),
    }
}

fn withdraw(args: &WithdrawArgs) -> Result<()> {
    let session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    let sessions_dir = store::sessions_dir()?;
    withdraw_todo(
        &sessions_dir,
        &session_id,
        args.seq,
        args.comment.as_deref(),
    )?;
    println!("Withdrew todo {}", args.seq);
    Ok(())
}

fn withdraw_todo(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    comment: Option<&str>,
) -> Result<()> {
    let comment = comment.map(str::trim).filter(|comment| !comment.is_empty());
    let _todo_lock = store::lock_agent_todo_resolution(sessions_dir, session_id)?;
    ensure!(
        store::resolve_agent_todo_from(
            sessions_dir,
            session_id,
            seq,
            store::TodoResolution::Withdrawn,
            comment,
        )?,
        "todo is already resolved or does not exist"
    );
    Ok(())
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

    use clap::Parser;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::{TodoCommands, record_todo, withdraw_todo};
    use crate::commands::agent::store::{self, TodoResolution};

    #[derive(Parser)]
    struct TodoCli {
        #[command(subcommand)]
        command: TodoCommands,
    }

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

    fn create_session(sessions_dir: &Path) {
        crate::commands::agent::store::session_store_revision_from(sessions_dir)
            .expect("session store should initialize");
        let connection = rusqlite::Connection::open(sessions_dir.join("sessions.db"))
            .expect("session database should open");
        connection
            .execute(
                "INSERT INTO sessions (session_id, data, rev) VALUES (?1, '{}', 0)",
                ["session-a"],
            )
            .expect("session row should be stored");
        connection
            .execute(
                "INSERT INTO sessions (session_id, data, rev) VALUES (?1, '{}', 0)",
                ["session-b"],
            )
            .expect("other session row should be stored");
    }

    #[rstest]
    fn records_a_todo_with_optional_detail(sessions_dir: TempDir) {
        create_session(sessions_dir.path());
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
    fn withdraws_only_the_selected_session_todo_with_a_comment(sessions_dir: TempDir) {
        create_session(sessions_dir.path());
        let session_a_seq =
            store::add_agent_todo_in(sessions_dir.path(), "session-a", "Check the page", None)
                .expect("todo should save");
        store::add_agent_todo_in(sessions_dir.path(), "session-b", "Other task", None)
            .expect("other session todo should save");
        withdraw_todo(
            sessions_dir.path(),
            "session-a",
            session_a_seq,
            Some("  The plan changed.  "),
        )
        .expect("todo should be withdrawn");

        let actual = ["session-a", "session-b"].map(|session_id| {
            store::agent_todos_from(sessions_dir.path(), session_id)
                .expect("todos should load")
                .into_iter()
                .map(|todo| (todo.title, todo.resolution, todo.comment))
                .collect::<Vec<_>>()
        });

        assert_eq!(
            actual,
            [
                vec![(
                    "Check the page".to_string(),
                    Some(TodoResolution::Withdrawn),
                    Some("The plan changed.".to_string()),
                )],
                vec![("Other task".to_string(), None, None)],
            ],
        );
    }

    #[rstest]
    fn parses_the_withdraw_command_and_optional_comment() {
        let cli =
            TodoCli::try_parse_from(["todo", "withdraw", "3", "--comment", "No longer needed."])
                .expect("withdraw arguments should parse");

        let actual = match cli.command {
            TodoCommands::Withdraw(args) => (args.seq, args.comment),
            TodoCommands::Add(_) => (0, None),
        };

        assert_eq!(actual, (3, Some("No longer needed.".to_string())));
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
