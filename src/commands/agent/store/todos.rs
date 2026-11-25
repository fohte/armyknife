use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::{TodoResolution, database, sessions_dir};

pub(crate) fn resolve_agent_todo_from_at(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    resolution: TodoResolution,
    comment: Option<&str>,
    completed_at: DateTime<Utc>,
) -> Result<bool> {
    database::resolve_agent_todo_at(
        sessions_dir,
        session_id,
        seq,
        resolution,
        comment,
        completed_at,
    )
}

pub(crate) fn resolve_agent_todo_at(
    session_id: &str,
    seq: i64,
    resolution: TodoResolution,
    comment: Option<&str>,
    completed_at: DateTime<Utc>,
) -> Result<bool> {
    resolve_agent_todo_from_at(
        &sessions_dir()?,
        session_id,
        seq,
        resolution,
        comment,
        completed_at,
    )
}
