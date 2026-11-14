use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{open_connection, validate_session_id};

// Failed message delivery must not suppress a later prompt indefinitely.
const PENDING_PROMPT_TTL_MILLIS: i64 = 10 * 60 * 1000;

pub(crate) fn mark_pending_initial_prompt(sessions_dir: &Path, session_id: &str) -> Result<()> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = Utc::now().timestamp_millis();
    transaction.execute(
        "DELETE FROM pending_initial_prompt WHERE created_at < ?1",
        [now - PENDING_PROMPT_TTL_MILLIS],
    )?;
    transaction.execute(
        "INSERT OR REPLACE INTO pending_initial_prompt (session_id, created_at)
         VALUES (?1, ?2)",
        params![session_id, now],
    )?;
    transaction.commit()?;
    Ok(())
}

pub(crate) fn clear_pending_initial_prompt(sessions_dir: &Path, session_id: &str) -> Result<()> {
    validate_session_id(session_id)?;
    let connection = open_connection(sessions_dir)?;
    connection.execute(
        "DELETE FROM pending_initial_prompt WHERE session_id = ?1",
        [session_id],
    )?;
    Ok(())
}

pub(super) fn consume_pending_initial_prompt(
    connection: &Connection,
    session_id: &str,
) -> Result<bool> {
    let created_at = connection
        .query_row(
            "SELECT created_at FROM pending_initial_prompt WHERE session_id = ?1",
            [session_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    connection.execute(
        "DELETE FROM pending_initial_prompt WHERE session_id = ?1",
        [session_id],
    )?;

    let now = Utc::now().timestamp_millis();
    Ok(created_at.is_some_and(|created_at| created_at >= now - PENDING_PROMPT_TTL_MILLIS))
}
