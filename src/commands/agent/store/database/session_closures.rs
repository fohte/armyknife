use std::collections::HashSet;
use std::path::Path;

use super::{open_connection, validate_session_id};
use anyhow::Result;

pub(crate) fn mark_session_closed(sessions_dir: &Path, session_id: &str) -> Result<()> {
    validate_session_id(session_id)?;
    let connection = open_connection(sessions_dir)?;
    connection.execute(
        "INSERT OR IGNORE INTO agent_session_close (session_id) VALUES (?1)",
        [session_id],
    )?;
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

        assert_eq!(
            closed_session_ids(sessions_dir.path()).expect("load close markers"),
            HashSet::from([session_id.to_string()]),
        );
    }
}
