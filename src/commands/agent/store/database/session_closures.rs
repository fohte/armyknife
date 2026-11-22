use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use rusqlite::TransactionBehavior;

use super::{open_connection, validate_session_id};

pub(crate) fn mark_session_closed(sessions_dir: &Path, session_id: &str) -> Result<()> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let inserted = transaction.execute(
        "INSERT OR IGNORE INTO agent_session_close (session_id) VALUES (?1)",
        [session_id],
    )?;
    if inserted > 0 {
        super::increment_revision(&transaction)?;
    }
    transaction.commit()?;
    Ok(())
}

pub(crate) fn clear_session_closed(sessions_dir: &Path, session_id: &str) -> Result<()> {
    validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(());
    }

    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let removed = transaction.execute(
        "DELETE FROM agent_session_close WHERE session_id = ?1",
        [session_id],
    )?;
    if removed > 0 {
        super::increment_revision(&transaction)?;
    }
    transaction.commit()?;
    Ok(())
}

pub(crate) fn closed_session_ids(sessions_dir: &Path) -> Result<HashSet<String>> {
    if !sessions_dir.exists() {
        return Ok(HashSet::new());
    }

    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare("SELECT session_id FROM agent_session_close")?;
    let session_ids = statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    Ok(session_ids)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn closed_session_ids_are_persisted() {
        let sessions_dir = TempDir::new().expect("create session directory");
        let session_id = "fixture-session";

        mark_session_closed(sessions_dir.path(), session_id).expect("mark close");
        let revision_after_close = super::super::session_store_revision(sessions_dir.path())
            .expect("read revision after close");
        mark_session_closed(sessions_dir.path(), session_id).expect("repeat close marker");

        assert_eq!(
            (
                closed_session_ids(sessions_dir.path()).expect("load close markers"),
                revision_after_close,
                super::super::session_store_revision(sessions_dir.path())
                    .expect("read revision after repeated close marker"),
            ),
            (HashSet::from([session_id.to_string()]), 1, 1),
        );
    }

    #[test]
    fn deleting_a_session_removes_its_close_marker() {
        let sessions_dir = TempDir::new().expect("create session directory");
        let session_id = "fixture-session";
        let connection = open_connection(sessions_dir.path()).expect("open session database");
        connection
            .execute(
                "INSERT INTO sessions (session_id, data, rev) VALUES (?1, '{}', 0)",
                [session_id],
            )
            .expect("insert session row");
        drop(connection);

        mark_session_closed(sessions_dir.path(), session_id).expect("mark close");
        let connection = open_connection(sessions_dir.path()).expect("reopen session database");
        connection
            .execute("DELETE FROM sessions WHERE session_id = ?1", [session_id])
            .expect("delete session row");

        assert_eq!(
            closed_session_ids(sessions_dir.path()).expect("load close markers"),
            HashSet::new(),
        );
    }
}
