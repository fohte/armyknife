use std::path::Path;

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use rusqlite::{TransactionBehavior, params, params_from_iter};
use serde::{Deserialize, Serialize};

use super::{increment_revision, open_connection, validate_session_id};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentTodo {
    pub(crate) session_id: String,
    pub(crate) seq: i64,
    pub(crate) title: String,
    pub(crate) detail: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) completed_at: Option<DateTime<Utc>>,
    pub(crate) resolution: Option<TodoResolution>,
    pub(crate) comment: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TodoResolution {
    Completed,
    Rejected,
    Withdrawn,
}

impl TodoResolution {
    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Rejected => "rejected",
            Self::Withdrawn => "withdrawn",
        }
    }

    fn parse(value: Option<String>, completed_at: Option<&str>) -> Result<Option<Self>> {
        let resolution = match value.as_deref() {
            Some("completed") => Some(Self::Completed),
            Some("rejected") => Some(Self::Rejected),
            Some("withdrawn") => Some(Self::Withdrawn),
            Some(value) => anyhow::bail!("unknown todo resolution: {value}"),
            None if completed_at.is_some() => Some(Self::Completed),
            None => None,
        };
        Ok(resolution)
    }
}

pub(crate) fn add_agent_todo(
    sessions_dir: &Path,
    session_id: &str,
    title: &str,
    detail: Option<&str>,
) -> Result<i64> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let session_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
        [session_id],
        |row| row.get(0),
    )?;
    ensure!(session_exists, "session not found: {session_id}");
    let now = Utc::now().to_rfc3339();
    let seq: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM agent_todo WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO agent_todo (session_id, seq, title, detail, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![session_id, seq, title, detail, now],
    )?;
    increment_revision(&transaction)?;
    transaction.commit()?;
    Ok(seq)
}

pub(crate) fn agent_todos(sessions_dir: &Path, session_id: &str) -> Result<Vec<AgentTodo>> {
    validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare(
        "SELECT session_id, seq, title, detail, created_at, completed_at, resolution, comment
         FROM agent_todo WHERE session_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map([session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
        ))
    })?;
    rows.map(|row| {
        let (session_id, seq, title, detail, created_at, completed_at, resolution, comment) = row?;
        let resolution = TodoResolution::parse(resolution, completed_at.as_deref())?;
        Ok(AgentTodo {
            session_id,
            seq,
            title,
            detail,
            created_at: DateTime::parse_from_rfc3339(&created_at)
                .context("parsing todo created_at")?
                .with_timezone(&Utc),
            completed_at: completed_at
                .map(|value| {
                    DateTime::parse_from_rfc3339(&value)
                        .context("parsing todo completed_at")
                        .map(|parsed| parsed.with_timezone(&Utc))
                })
                .transpose()?,
            resolution,
            comment,
        })
    })
    .collect()
}

pub(crate) fn incomplete_agent_todo_titles(
    sessions_dir: &Path,
    session_ids: &[String],
) -> Result<Vec<(String, String)>> {
    if !sessions_dir.exists() || session_ids.is_empty() {
        return Ok(Vec::new());
    }

    let connection = open_connection(sessions_dir)?;
    let placeholders = (1..=session_ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!(
        "SELECT session_id, title FROM agent_todo
         WHERE completed_at IS NULL AND session_id IN ({placeholders})
         ORDER BY session_id, seq"
    );
    let mut statement = connection.prepare(&query)?;
    let rows = statement.query_map(params_from_iter(session_ids.iter()), |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(crate) fn resolve_agent_todo(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    resolution: TodoResolution,
    comment: Option<&str>,
) -> Result<bool> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE agent_todo SET completed_at = ?1, resolution = ?2, comment = ?3
         WHERE session_id = ?4 AND seq = ?5 AND completed_at IS NULL",
        params![
            Utc::now().to_rfc3339(),
            resolution.as_str(),
            comment,
            session_id,
            seq
        ],
    )?;
    if changed > 0 {
        increment_revision(&transaction)?;
    }
    transaction.commit()?;
    Ok(changed > 0)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::{
        AgentTodo, TodoResolution, add_agent_todo, agent_todos, incomplete_agent_todo_titles,
        open_connection, resolve_agent_todo,
    };
    use crate::commands::agent::store::session_store_revision_from;

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().expect("temporary sessions directory should be created")
    }

    fn create_session(sessions_dir: &Path, session_id: &str) {
        open_connection(sessions_dir)
            .expect("database should open")
            .execute(
                "INSERT INTO sessions (session_id, data, rev) VALUES (?1, '{}', 0)",
                [session_id],
            )
            .expect("session row should be stored");
    }

    fn normalize_timestamps(mut todos: Vec<AgentTodo>) -> Vec<AgentTodo> {
        let epoch = Utc
            .timestamp_opt(0, 0)
            .single()
            .expect("epoch should be valid");
        todos.iter_mut().for_each(|todo| {
            todo.created_at = epoch;
            todo.completed_at = todo.completed_at.map(|_| epoch);
        });
        todos
    }

    #[rstest]
    fn records_lists_and_completes_todos_for_their_sessions(sessions_dir: TempDir) {
        create_session(sessions_dir.path(), "session-a");
        create_session(sessions_dir.path(), "session-b");
        let first = add_agent_todo(sessions_dir.path(), "session-a", "First", None)
            .expect("todo should save");
        let second = add_agent_todo(sessions_dir.path(), "session-a", "Second", Some("Details"))
            .expect("todo should save");
        add_agent_todo(sessions_dir.path(), "session-b", "Other", None)
            .expect("other session todo should save");
        let changed = resolve_agent_todo(
            sessions_dir.path(),
            "session-a",
            first,
            TodoResolution::Completed,
            Some("Done"),
        )
        .expect("todo should complete");
        let todos = normalize_timestamps(
            agent_todos(sessions_dir.path(), "session-a").expect("todos should load"),
        );
        let incomplete = incomplete_agent_todo_titles(
            sessions_dir.path(),
            &["session-a".to_string(), "session-b".to_string()],
        )
        .expect("incomplete todo titles should load");
        let epoch = Utc
            .timestamp_opt(0, 0)
            .single()
            .expect("epoch should be valid");

        assert_eq!(
            (second, changed, todos, incomplete),
            (
                2,
                true,
                vec![
                    AgentTodo {
                        session_id: "session-a".to_string(),
                        seq: 1,
                        title: "First".to_string(),
                        detail: None,
                        created_at: epoch,
                        completed_at: Some(epoch),
                        resolution: Some(TodoResolution::Completed),
                        comment: Some("Done".to_string()),
                    },
                    AgentTodo {
                        session_id: "session-a".to_string(),
                        seq: 2,
                        title: "Second".to_string(),
                        detail: Some("Details".to_string()),
                        created_at: epoch,
                        completed_at: None,
                        resolution: None,
                        comment: None,
                    },
                ],
                vec![
                    ("session-a".to_string(), "Second".to_string()),
                    ("session-b".to_string(), "Other".to_string()),
                ],
            ),
        );
    }

    #[rstest]
    fn completion_advances_the_store_revision_only_once(sessions_dir: TempDir) {
        create_session(sessions_dir.path(), "session-a");
        let seq = add_agent_todo(sessions_dir.path(), "session-a", "Task", None)
            .expect("todo should save");
        let before =
            session_store_revision_from(sessions_dir.path()).expect("revision should load");
        let first = resolve_agent_todo(
            sessions_dir.path(),
            "session-a",
            seq,
            TodoResolution::Completed,
            None,
        )
        .expect("todo should complete");
        let after_first =
            session_store_revision_from(sessions_dir.path()).expect("revision should load");
        let second = resolve_agent_todo(
            sessions_dir.path(),
            "session-a",
            seq,
            TodoResolution::Completed,
            Some("Again"),
        )
        .expect("repeat completion should be handled");
        let after_second =
            session_store_revision_from(sessions_dir.path()).expect("revision should load");

        assert_eq!(
            (
                first,
                after_first - before,
                second,
                after_second - after_first
            ),
            (true, 1, false, 0)
        );
    }

    #[rstest]
    fn reads_pre_resolution_completed_rows_as_completed(sessions_dir: TempDir) {
        create_session(sessions_dir.path(), "session-a");
        let seq = add_agent_todo(sessions_dir.path(), "session-a", "Task", None)
            .expect("todo should save");
        resolve_agent_todo(
            sessions_dir.path(),
            "session-a",
            seq,
            TodoResolution::Completed,
            Some("Done"),
        )
        .expect("todo should complete");
        open_connection(sessions_dir.path())
            .expect("database should open")
            .execute(
                "UPDATE agent_todo SET resolution = NULL WHERE session_id = ?1 AND seq = ?2",
                rusqlite::params!["session-a", seq],
            )
            .expect("legacy row should be represented");

        let todos = agent_todos(sessions_dir.path(), "session-a").expect("todos should load");
        let todo = todos
            .first()
            .map(|todo| (todo.resolution, todo.comment.as_deref()));

        assert_eq!(todo, Some((Some(TodoResolution::Completed), Some("Done"))),);
    }

    #[rstest]
    fn todo_is_not_added_after_its_session_was_deleted(sessions_dir: TempDir) {
        create_session(sessions_dir.path(), "session-a");
        let removed = super::super::delete_session(sessions_dir.path(), "session-a")
            .expect("session should be deleted");

        let result = add_agent_todo(sessions_dir.path(), "session-a", "Orphan", None);
        let todos = agent_todos(sessions_dir.path(), "session-a").unwrap();

        assert_eq!(
            (removed, result.unwrap_err().to_string(), todos),
            (true, "session not found: session-a".to_string(), Vec::new()),
        );
    }

    #[rstest]
    #[case::incomplete_todo(false, false, true)]
    #[case::all_todos_completed(true, true, false)]
    fn deleting_a_session_preserves_todos_until_all_are_completed(
        sessions_dir: TempDir,
        #[case] complete_second_todo: bool,
        #[case] expected_removed: bool,
        #[case] expected_session_exists: bool,
    ) {
        create_session(sessions_dir.path(), "session-a");
        let first_seq = add_agent_todo(sessions_dir.path(), "session-a", "First", None).unwrap();
        let second_seq = add_agent_todo(sessions_dir.path(), "session-a", "Second", None).unwrap();
        resolve_agent_todo(
            sessions_dir.path(),
            "session-a",
            first_seq,
            TodoResolution::Completed,
            Some("Done"),
        )
        .unwrap();
        if complete_second_todo {
            resolve_agent_todo(
                sessions_dir.path(),
                "session-a",
                second_seq,
                TodoResolution::Completed,
                Some("Done"),
            )
            .unwrap();
        }

        let removed = super::super::delete_session(sessions_dir.path(), "session-a").unwrap();
        let session_exists: bool = open_connection(sessions_dir.path())
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
                ["session-a"],
                |row| row.get(0),
            )
            .unwrap();
        let todos = normalize_timestamps(agent_todos(sessions_dir.path(), "session-a").unwrap());
        let epoch = Utc.timestamp_opt(0, 0).single().unwrap();
        let expected_todos = if complete_second_todo {
            Vec::new()
        } else {
            vec![
                AgentTodo {
                    session_id: "session-a".to_string(),
                    seq: first_seq,
                    title: "First".to_string(),
                    detail: None,
                    created_at: epoch,
                    completed_at: Some(epoch),
                    resolution: Some(TodoResolution::Completed),
                    comment: Some("Done".to_string()),
                },
                AgentTodo {
                    session_id: "session-a".to_string(),
                    seq: second_seq,
                    title: "Second".to_string(),
                    detail: None,
                    created_at: epoch,
                    completed_at: None,
                    resolution: None,
                    comment: None,
                },
            ]
        };

        assert_eq!(
            (removed, session_exists, todos),
            (expected_removed, expected_session_exists, expected_todos),
        );
    }
}
