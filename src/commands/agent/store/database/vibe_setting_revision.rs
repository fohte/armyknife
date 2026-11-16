use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn current(connection: &Connection, session_id: &str) -> Result<Option<String>> {
    Ok(connection
        .query_row(
            "SELECT revision FROM session_vibe_setting_revision WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )
        .optional()?)
}

pub(super) fn rotate(connection: &Connection, session_id: &str) -> Result<String> {
    let revision = uuid::Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO session_vibe_setting_revision (session_id, revision)
         VALUES (?1, ?2)
         ON CONFLICT(session_id) DO UPDATE SET revision = excluded.revision",
        params![session_id, revision],
    )?;
    Ok(revision)
}
