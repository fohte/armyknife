use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::Value;

use super::{increment_revision, open_connection, validate_session_id};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PermissionDeniedOperation {
    pub(crate) session_id: String,
    pub(crate) seq: i64,
    pub(crate) tool_use_id: Option<String>,
    pub(crate) tool_name: String,
    pub(crate) tool_input: Value,
    pub(crate) reason: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) resolved_at: Option<DateTime<Utc>>,
}

impl PermissionDeniedOperation {
    pub(crate) fn has_classifier_verdict(&self) -> bool {
        self.reason.as_deref().is_some_and(|reason| {
            !reason.trim().is_empty()
                && reason != "Classifier unavailable"
                && reason != "No reason provided"
                && !reason.starts_with("Auto mode could not evaluate this action")
        })
    }
}

pub(crate) fn add_permission_denied(
    sessions_dir: &Path,
    session_id: &str,
    tool_use_id: Option<&str>,
    tool_name: &str,
    tool_input: &Value,
    reason: Option<&str>,
) -> Result<i64> {
    validate_session_id(session_id)?;
    let input = serde_json::to_string(tool_input).context("serializing denied tool input")?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(tool_use_id) = tool_use_id {
        let existing = transaction
            .query_row(
                "SELECT seq FROM agent_permission_denied
                 WHERE session_id = ?1 AND tool_use_id = ?2",
                params![session_id, tool_use_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(seq) = existing {
            transaction.commit()?;
            return Ok(seq);
        }
    }
    let seq: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM agent_permission_denied WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO agent_permission_denied
            (session_id, seq, tool_use_id, tool_name, tool_input, reason, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            session_id,
            seq,
            tool_use_id,
            tool_name,
            input,
            reason,
            Utc::now().to_rfc3339()
        ],
    )?;
    increment_revision(&transaction)?;
    transaction.commit()?;
    Ok(seq)
}

pub(crate) fn permission_denied_operations(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<Vec<PermissionDeniedOperation>> {
    validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }
    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare(
        "SELECT session_id, seq, tool_use_id, tool_name, tool_input, reason, created_at, resolved_at
         FROM agent_permission_denied WHERE session_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map([session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?,
        ))
    })?;
    rows.map(|row| {
        let (session_id, seq, tool_use_id, tool_name, tool_input, reason, created_at, resolved_at) =
            row?;
        Ok(PermissionDeniedOperation {
            session_id,
            seq,
            tool_use_id,
            tool_name,
            tool_input: serde_json::from_str(&tool_input).context("parsing denied tool input")?,
            reason,
            created_at: DateTime::parse_from_rfc3339(&created_at)
                .context("parsing denied operation created_at")?
                .with_timezone(&Utc),
            resolved_at: resolved_at
                .map(|value| {
                    DateTime::parse_from_rfc3339(&value)
                        .context("parsing denied operation resolved_at")
                        .map(|parsed| parsed.with_timezone(&Utc))
                })
                .transpose()?,
        })
    })
    .collect()
}

pub(crate) fn resolve_permission_denied(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
) -> Result<bool> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE agent_permission_denied SET resolved_at = ?1
         WHERE session_id = ?2 AND seq = ?3 AND resolved_at IS NULL",
        params![Utc::now().to_rfc3339(), session_id, seq],
    )?;
    if changed > 0 {
        increment_revision(&transaction)?;
    }
    transaction.commit()?;
    Ok(changed > 0)
}
