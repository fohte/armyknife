use std::path::Path;

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{increment_revision, open_connection, validate_session_id};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentFile {
    pub(crate) id: String,
    pub(crate) position: i64,
    pub(crate) name: String,
    pub(crate) caption: Option<String>,
    pub(crate) mime_type: Option<String>,
    pub(crate) size: i64,
    pub(crate) created_at: DateTime<Utc>,
}

pub(crate) fn insert_agent_files(
    sessions_dir: &Path,
    session_id: &str,
    files: &[AgentFile],
) -> Result<()> {
    validate_session_id(session_id)?;
    ensure!(!files.is_empty(), "at least one file record is required");
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let session_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
        [session_id],
        |row| row.get(0),
    )?;
    ensure!(session_exists, "session not found: {session_id}");
    for file in files {
        transaction.execute(
            "INSERT INTO agent_file (session_id, id, position, name, caption, mime_type, size, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session_id,
                file.id,
                file.position,
                file.name,
                file.caption,
                file.mime_type,
                file.size,
                file.created_at.to_rfc3339(),
            ],
        )?;
    }
    increment_revision(&transaction)?;
    transaction.commit()?;
    Ok(())
}

pub(crate) fn agent_files(sessions_dir: &Path, session_id: &str) -> Result<Vec<AgentFile>> {
    validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }
    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare(
        "SELECT id, position, name, caption, mime_type, size, created_at
         FROM agent_file WHERE session_id = ?1 ORDER BY created_at, position",
    )?;
    let rows = statement.query_map([session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    rows.map(|row| {
        let (id, position, name, caption, mime_type, size, created_at) = row?;
        Ok(AgentFile {
            id,
            position,
            name,
            caption,
            mime_type,
            size,
            created_at: DateTime::parse_from_rfc3339(&created_at)
                .context("parsing agent file created_at")?
                .with_timezone(&Utc),
        })
    })
    .collect()
}

pub(crate) fn agent_file(
    sessions_dir: &Path,
    session_id: &str,
    file_id: &str,
) -> Result<Option<AgentFile>> {
    validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(None);
    }
    let connection = open_connection(sessions_dir)?;
    let file = connection
        .query_row(
            "SELECT id, position, name, caption, mime_type, size, created_at
             FROM agent_file WHERE session_id = ?1 AND id = ?2",
            params![session_id, file_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()?;
    file.map(
        |(id, position, name, caption, mime_type, size, created_at)| {
            Ok(AgentFile {
                id,
                position,
                name,
                caption,
                mime_type,
                size,
                created_at: DateTime::parse_from_rfc3339(&created_at)
                    .context("parsing agent file created_at")?
                    .with_timezone(&Utc),
            })
        },
    )
    .transpose()
}
