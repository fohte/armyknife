use std::fs::File;
use std::path::Path;

use super::WithdrawalReason;
use super::resolution_lock::lock_resolution_file;
use super::{bump_session_revision, open_connection, validate_session_id};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, TransactionBehavior, params, params_from_iter};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentTodo {
    pub(crate) session_id: String,
    pub(crate) seq: i64,
    pub(crate) title: String,
    pub(crate) detail: Option<String>,
    pub(crate) folded: bool,
    pub(crate) added_folded: bool,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) completed_at: Option<DateTime<Utc>>,
    pub(crate) resolution: Option<TodoResolution>,
    pub(crate) withdrawal_reason: Option<WithdrawalReason>,
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

pub(crate) fn lock_agent_todo_resolution(sessions_dir: &Path, session_id: &str) -> Result<File> {
    validate_session_id(session_id)?;
    lock_resolution_file(sessions_dir, "todo-resolution-locks", session_id)
}

pub(crate) fn has_incomplete_todos(connection: &Connection, session_id: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM agent_todo
            WHERE session_id = ?1 AND completed_at IS NULL
        )",
        [session_id],
        |row| row.get(0),
    )?)
}

pub(crate) fn add_agent_todo(
    sessions_dir: &Path,
    session_id: &str,
    title: &str,
    detail: Option<&str>,
    folded: bool,
) -> Result<i64> {
    validate_session_id(session_id)?;
    // Cleanup holds this lock through session deletion so a new todo cannot race with withdrawal.
    let _todo_lock = lock_agent_todo_resolution(sessions_dir, session_id)?;
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
        "INSERT INTO agent_todo (session_id, seq, title, detail, created_at, folded, added_folded)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![session_id, seq, title, detail, now, folded],
    )?;
    bump_session_revision(&transaction, session_id)?;
    transaction.commit()?;
    Ok(seq)
}

pub(crate) fn edit_agent_todo(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    title: &str,
    detail: Option<&str>,
    folded: bool,
) -> Result<bool> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE agent_todo SET title = ?1, detail = ?2, folded = ?3
         WHERE session_id = ?4 AND seq = ?5 AND completed_at IS NULL",
        params![title, detail, folded, session_id, seq],
    )?;
    if changed > 0 {
        bump_session_revision(&transaction, session_id)?;
    }
    transaction.commit()?;
    Ok(changed > 0)
}

pub(crate) fn agent_todos(sessions_dir: &Path, session_id: &str) -> Result<Vec<AgentTodo>> {
    validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare(
        "SELECT session_id, seq, title, detail, folded, added_folded, created_at, completed_at, resolution, withdrawal_reason, comment
         FROM agent_todo WHERE session_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map([session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, bool>(4)?,
            row.get::<_, bool>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, Option<String>>(10)?,
        ))
    })?;
    rows.map(|row| {
        let (
            session_id,
            seq,
            title,
            detail,
            folded,
            added_folded,
            created_at,
            completed_at,
            resolution,
            withdrawal_reason,
            comment,
        ) = row?;
        let resolution = TodoResolution::parse(resolution, completed_at.as_deref())?;
        Ok(AgentTodo {
            session_id,
            seq,
            title,
            detail,
            folded,
            added_folded,
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
            withdrawal_reason: WithdrawalReason::parse(withdrawal_reason)?,
            comment,
        })
    })
    .collect()
}

pub(crate) fn incomplete_agent_todo_titles(
    sessions_dir: &Path,
    session_ids: &[String],
) -> Result<Vec<(String, String, bool)>> {
    if !sessions_dir.exists() || session_ids.is_empty() {
        return Ok(Vec::new());
    }

    let connection = open_connection(sessions_dir)?;
    let placeholders = (1..=session_ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!(
        "SELECT session_id, title, folded FROM agent_todo
         WHERE completed_at IS NULL AND session_id IN ({placeholders})
         ORDER BY session_id, seq"
    );
    let mut statement = connection.prepare(&query)?;
    let rows = statement.query_map(params_from_iter(session_ids.iter()), |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(crate) fn set_agent_todo_folded(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    folded: bool,
) -> Result<bool> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let pending: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_todo
         WHERE session_id = ?1 AND seq = ?2 AND completed_at IS NULL)",
        params![session_id, seq],
        |row| row.get(0),
    )?;
    let changed = transaction.execute(
        "UPDATE agent_todo SET folded = ?1
         WHERE session_id = ?2 AND seq = ?3 AND completed_at IS NULL AND folded != ?1",
        params![folded, session_id, seq],
    )?;
    if changed > 0 {
        bump_session_revision(&transaction, session_id)?;
    }
    transaction.commit()?;
    Ok(pending)
}

#[cfg(test)]
pub(crate) fn resolve_agent_todo(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    resolution: TodoResolution,
    comment: Option<&str>,
) -> Result<bool> {
    resolve_agent_todo_at(
        sessions_dir,
        session_id,
        seq,
        resolution,
        comment,
        Utc::now(),
    )
}

pub(crate) fn resolve_agent_todo_at(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    resolution: TodoResolution,
    comment: Option<&str>,
    completed_at: DateTime<Utc>,
) -> Result<bool> {
    resolve_agent_todo_at_with_reason(
        sessions_dir,
        session_id,
        seq,
        resolution,
        comment,
        None,
        completed_at,
    )
}

pub(crate) fn resolve_agent_todo_at_with_reason(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    resolution: TodoResolution,
    comment: Option<&str>,
    withdrawal_reason: Option<WithdrawalReason>,
    completed_at: DateTime<Utc>,
) -> Result<bool> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE agent_todo SET completed_at = ?1, resolution = ?2, comment = ?3, withdrawal_reason = ?4
         WHERE session_id = ?5 AND seq = ?6 AND completed_at IS NULL",
        params![
            completed_at.to_rfc3339(),
            resolution.as_str(),
            comment,
            withdrawal_reason.map(WithdrawalReason::as_str),
            session_id,
            seq
        ],
    )?;
    if changed > 0 {
        bump_session_revision(&transaction, session_id)?;
    }
    transaction.commit()?;
    Ok(changed > 0)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::super::WithdrawalReason;
    use super::{
        AgentTodo, TodoResolution, add_agent_todo, agent_todos, incomplete_agent_todo_titles,
        lock_agent_todo_resolution, open_connection, resolve_agent_todo, resolve_agent_todo_at,
        resolve_agent_todo_at_with_reason, set_agent_todo_folded,
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

    #[rstest]
    #[case::session_ended(WithdrawalReason::SessionEnded)]
    #[case::worktree_cleanup(WithdrawalReason::WorktreeCleanup)]
    #[case::manual(WithdrawalReason::Manual)]
    fn todo_withdrawal_persists_its_reason(
        sessions_dir: TempDir,
        #[case] reason: WithdrawalReason,
    ) {
        create_session(sessions_dir.path(), "session-a");
        let seq = add_agent_todo(
            sessions_dir.path(),
            "session-a",
            "Task",
            Some("Details"),
            false,
        )
        .expect("todo should be stored");
        let resolved_at = Utc
            .timestamp_opt(100, 0)
            .single()
            .expect("time should be valid");

        resolve_agent_todo_at_with_reason(
            sessions_dir.path(),
            "session-a",
            seq,
            TodoResolution::Withdrawn,
            Some("Withdrawal note"),
            Some(reason),
            resolved_at,
        )
        .expect("todo should be withdrawn");

        let mut actual = agent_todos(sessions_dir.path(), "session-a")
            .expect("todo should load")
            .remove(0);
        actual.created_at = Utc
            .timestamp_opt(0, 0)
            .single()
            .expect("epoch should be valid");
        actual.completed_at = Some(resolved_at);
        assert_eq!(
            actual,
            AgentTodo {
                session_id: "session-a".to_string(),
                seq: 1,
                title: "Task".to_string(),
                detail: Some("Details".to_string()),
                folded: false,
                added_folded: false,
                created_at: Utc
                    .timestamp_opt(0, 0)
                    .single()
                    .expect("epoch should be valid"),
                completed_at: Some(resolved_at),
                resolution: Some(TodoResolution::Withdrawn),
                withdrawal_reason: Some(reason),
                comment: Some("Withdrawal note".to_string()),
            },
        );
    }

    #[rstest]
    fn todo_resolution_lock_keeps_its_existing_file_name(sessions_dir: TempDir) {
        let _lock = lock_agent_todo_resolution(sessions_dir.path(), "session-a")
            .expect("todo resolution lock should be acquired");
        let lock_files = std::fs::read_dir(sessions_dir.path().join("todo-resolution-locks"))
            .expect("todo resolution lock directory should exist")
            .map(|entry| {
                entry
                    .expect("todo resolution lock should be readable")
                    .file_name()
                    .into_string()
                    .expect("todo resolution lock name should be valid UTF-8")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            lock_files,
            vec!["fa57a52dbf08190218529730a3e99db6946c6c29220fb6e0551e21598b0b05db.lock"],
        );
    }

    #[rstest]
    fn competing_resolutions_persist_only_the_winner(sessions_dir: TempDir) {
        create_session(sessions_dir.path(), "session-a");
        let seq = add_agent_todo(sessions_dir.path(), "session-a", "Task", None, false)
            .expect("todo should be stored");
        let start = Arc::new(Barrier::new(2));
        let thread_start = Arc::clone(&start);
        let thread_sessions_dir = sessions_dir.path().to_path_buf();
        let rejection = thread::spawn(move || {
            thread_start.wait();
            let _todo_lock = lock_agent_todo_resolution(&thread_sessions_dir, "session-a")?;
            resolve_agent_todo(
                &thread_sessions_dir,
                "session-a",
                seq,
                TodoResolution::Rejected,
                Some("No longer needed"),
            )
        });

        start.wait();
        let _todo_lock = lock_agent_todo_resolution(sessions_dir.path(), "session-a")
            .expect("withdrawal should acquire the session lock");
        let withdrawn = resolve_agent_todo(
            sessions_dir.path(),
            "session-a",
            seq,
            TodoResolution::Withdrawn,
            Some("Plan changed"),
        )
        .expect("withdrawal should save");
        drop(_todo_lock);
        let rejected = rejection
            .join()
            .expect("rejection thread should finish")
            .expect("rejection attempt should finish");
        let stored_resolution = agent_todos(sessions_dir.path(), "session-a")
            .expect("todo should load")
            .first()
            .and_then(|todo| todo.resolution);
        let expected_resolution = if withdrawn {
            Some(TodoResolution::Withdrawn)
        } else {
            Some(TodoResolution::Rejected)
        };

        assert_eq!(
            (withdrawn, rejected, stored_resolution),
            (withdrawn, !withdrawn, expected_resolution),
        );
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
        let first = add_agent_todo(sessions_dir.path(), "session-a", "First", None, false)
            .expect("todo should save");
        let second = add_agent_todo(
            sessions_dir.path(),
            "session-a",
            "Second",
            Some("Details"),
            false,
        )
        .expect("todo should save");
        add_agent_todo(sessions_dir.path(), "session-b", "Other", None, true)
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
                        folded: false,
                        added_folded: false,
                        created_at: epoch,
                        completed_at: Some(epoch),
                        resolution: Some(TodoResolution::Completed),
                        withdrawal_reason: None,
                        comment: Some("Done".to_string()),
                    },
                    AgentTodo {
                        session_id: "session-a".to_string(),
                        seq: 2,
                        title: "Second".to_string(),
                        detail: Some("Details".to_string()),
                        folded: false,
                        added_folded: false,
                        created_at: epoch,
                        completed_at: None,
                        resolution: None,
                        withdrawal_reason: None,
                        comment: None,
                    },
                ],
                vec![
                    ("session-a".to_string(), "Second".to_string(), false),
                    ("session-b".to_string(), "Other".to_string(), true),
                ],
            ),
        );
    }

    #[rstest]
    fn stores_the_completion_time_supplied_by_the_watch_web_action(sessions_dir: TempDir) {
        create_session(sessions_dir.path(), "session-a");
        let seq = add_agent_todo(sessions_dir.path(), "session-a", "First", None, false)
            .expect("todo should save");
        let completed_at = Utc
            .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
            .single()
            .expect("fixture time should be valid");
        resolve_agent_todo_at(
            sessions_dir.path(),
            "session-a",
            seq,
            TodoResolution::Completed,
            None,
            completed_at,
        )
        .expect("todo should complete");

        let epoch = Utc
            .timestamp_opt(0, 0)
            .single()
            .expect("epoch should be valid");
        let todo = agent_todos(sessions_dir.path(), "session-a")
            .expect("todo should load")
            .into_iter()
            .map(|todo| AgentTodo {
                created_at: epoch,
                ..todo
            })
            .collect::<Vec<_>>();

        assert_eq!(
            todo,
            vec![AgentTodo {
                session_id: "session-a".to_string(),
                seq,
                title: "First".to_string(),
                detail: None,
                folded: false,
                added_folded: false,
                created_at: epoch,
                completed_at: Some(completed_at),
                resolution: Some(TodoResolution::Completed),
                withdrawal_reason: None,
                comment: None,
            }],
        );
    }

    #[rstest]
    fn folded_todo_state_persists_without_losing_its_added_marker(sessions_dir: TempDir) {
        create_session(sessions_dir.path(), "session-a");
        let seq = add_agent_todo(sessions_dir.path(), "session-a", "Check later", None, true)
            .expect("folded todo should save");
        let before =
            session_store_revision_from(sessions_dir.path()).expect("revision should load");

        let unchanged = set_agent_todo_folded(sessions_dir.path(), "session-a", seq, true)
            .expect("existing todo should remain folded");
        let changed = set_agent_todo_folded(sessions_dir.path(), "session-a", seq, false)
            .expect("existing todo should unfold");
        let after = session_store_revision_from(sessions_dir.path()).expect("revision should load");
        let todo = agent_todos(sessions_dir.path(), "session-a")
            .expect("todo should load")
            .into_iter()
            .next();

        assert_eq!(
            (
                unchanged,
                changed,
                after - before,
                todo.map(|todo| (todo.folded, todo.added_folded))
            ),
            (true, true, 1, Some((false, true))),
        );
    }

    #[rstest]
    fn completion_advances_the_store_revision_only_once(sessions_dir: TempDir) {
        create_session(sessions_dir.path(), "session-a");
        let seq = add_agent_todo(sessions_dir.path(), "session-a", "Task", None, false)
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
        let seq = add_agent_todo(sessions_dir.path(), "session-a", "Task", None, false)
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

        let result = add_agent_todo(sessions_dir.path(), "session-a", "Orphan", None, false);
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
        let first_seq =
            add_agent_todo(sessions_dir.path(), "session-a", "First", None, false).unwrap();
        let second_seq =
            add_agent_todo(sessions_dir.path(), "session-a", "Second", None, false).unwrap();
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
                    folded: false,
                    added_folded: false,
                    created_at: epoch,
                    completed_at: Some(epoch),
                    resolution: Some(TodoResolution::Completed),
                    withdrawal_reason: None,
                    comment: Some("Done".to_string()),
                },
                AgentTodo {
                    session_id: "session-a".to_string(),
                    seq: second_seq,
                    title: "Second".to_string(),
                    detail: None,
                    folded: false,
                    added_folded: false,
                    created_at: epoch,
                    completed_at: None,
                    resolution: None,
                    withdrawal_reason: None,
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
