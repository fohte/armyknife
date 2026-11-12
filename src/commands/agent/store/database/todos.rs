use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{TransactionBehavior, params, params_from_iter};

use super::{increment_revision, open_connection, validate_session_id};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentTodo {
    pub(crate) session_id: String,
    pub(crate) seq: i64,
    pub(crate) title: String,
    pub(crate) detail: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) completed_at: Option<DateTime<Utc>>,
    pub(crate) comment: Option<String>,
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
        "SELECT session_id, seq, title, detail, created_at, completed_at, comment
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
        ))
    })?;
    rows.map(|row| {
        let (session_id, seq, title, detail, created_at, completed_at, comment) = row?;
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

pub(crate) fn complete_agent_todo(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    comment: Option<&str>,
) -> Result<bool> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE agent_todo SET completed_at = ?1, comment = ?2
         WHERE session_id = ?3 AND seq = ?4 AND completed_at IS NULL",
        params![Utc::now().to_rfc3339(), comment, session_id, seq],
    )?;
    if changed > 0 {
        increment_revision(&transaction)?;
    }
    transaction.commit()?;
    Ok(changed > 0)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::{
        AgentTodo, add_agent_todo, agent_todos, complete_agent_todo, incomplete_agent_todo_titles,
        open_connection,
    };
    use crate::commands::agent::store::session_store_revision_from;

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().expect("temporary sessions directory should be created")
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
        let first = add_agent_todo(sessions_dir.path(), "session-a", "First", None)
            .expect("todo should save");
        let second = add_agent_todo(sessions_dir.path(), "session-a", "Second", Some("Details"))
            .expect("todo should save");
        add_agent_todo(sessions_dir.path(), "session-b", "Other", None)
            .expect("other session todo should save");
        let changed = complete_agent_todo(sessions_dir.path(), "session-a", first, Some("Done"))
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
                        comment: Some("Done".to_string()),
                    },
                    AgentTodo {
                        session_id: "session-a".to_string(),
                        seq: 2,
                        title: "Second".to_string(),
                        detail: Some("Details".to_string()),
                        created_at: epoch,
                        completed_at: None,
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
        let seq = add_agent_todo(sessions_dir.path(), "session-a", "Task", None)
            .expect("todo should save");
        let before =
            session_store_revision_from(sessions_dir.path()).expect("revision should load");
        let first = complete_agent_todo(sessions_dir.path(), "session-a", seq, None)
            .expect("todo should complete");
        let after_first =
            session_store_revision_from(sessions_dir.path()).expect("revision should load");
        let second = complete_agent_todo(sessions_dir.path(), "session-a", seq, Some("Again"))
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
    #[case::incomplete_todo(false, false, true)]
    #[case::all_todos_completed(true, true, false)]
    fn deleting_a_session_preserves_todos_until_all_are_completed(
        sessions_dir: TempDir,
        #[case] complete_second_todo: bool,
        #[case] expected_removed: bool,
        #[case] expected_session_exists: bool,
    ) {
        let connection = open_connection(sessions_dir.path()).expect("database should open");
        connection
            .execute(
                "INSERT INTO sessions (session_id, data, rev) VALUES (?1, '{}', 0)",
                ["session-a"],
            )
            .expect("session row should be stored");
        let first_seq = add_agent_todo(sessions_dir.path(), "session-a", "First", None).unwrap();
        let second_seq = add_agent_todo(sessions_dir.path(), "session-a", "Second", None).unwrap();
        complete_agent_todo(sessions_dir.path(), "session-a", first_seq, Some("Done")).unwrap();
        if complete_second_todo {
            complete_agent_todo(sessions_dir.path(), "session-a", second_seq, Some("Done"))
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
                    comment: Some("Done".to_string()),
                },
                AgentTodo {
                    session_id: "session-a".to_string(),
                    seq: second_seq,
                    title: "Second".to_string(),
                    detail: None,
                    created_at: epoch,
                    completed_at: None,
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
