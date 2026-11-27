use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};

use super::{open_connection, validate_session_id};

pub(crate) fn record_agent_response_at(
    connection: &Connection,
    session_id: &str,
    responded_at: DateTime<Utc>,
) -> Result<()> {
    validate_session_id(session_id)?;
    connection.execute(
        "INSERT INTO agent_session_activity (session_id, last_responded_at)
         VALUES (?1, ?2)
         ON CONFLICT(session_id) DO UPDATE SET
             last_responded_at = MAX(last_responded_at, excluded.last_responded_at)",
        params![session_id, responded_at.timestamp_micros()],
    )?;
    Ok(())
}

pub(crate) fn agent_response_timestamps(
    sessions_dir: &Path,
) -> Result<HashMap<String, DateTime<Utc>>> {
    if !sessions_dir.exists() {
        return Ok(HashMap::new());
    }

    let connection = open_connection(sessions_dir)?;
    let mut statement =
        connection.prepare("SELECT session_id, last_responded_at FROM agent_session_activity")?;
    let mut rows = statement.query([])?;
    let mut timestamps = HashMap::new();
    while let Some(row) = rows.next()? {
        let session_id: String = row.get(0)?;
        let timestamp_micros: i64 = row.get(1)?;
        let responded_at = DateTime::from_timestamp_micros(timestamp_micros)
            .ok_or_else(|| anyhow!("invalid agent response timestamp for session {session_id}"))?;
        timestamps.insert(session_id, responded_at);
    }

    Ok(timestamps)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;

    #[fixture]
    fn sessions_dir() -> TempDir {
        TempDir::new().expect("temporary sessions directory should be created")
    }

    #[rstest]
    fn response_timestamp_keeps_the_latest_recorded_agent_response(
        sessions_dir: TempDir,
    ) -> Result<()> {
        let older = Utc.timestamp_opt(900, 0).single().expect("valid timestamp");
        let latest = Utc.timestamp_opt(950, 0).single().expect("valid timestamp");
        let newer_prompt = Utc.timestamp_opt(925, 0).single().expect("valid timestamp");
        let connection = open_connection(sessions_dir.path())?;
        record_agent_response_at(&connection, "fixture-session-maple", latest)?;
        record_agent_response_at(&connection, "fixture-session-maple", older)?;
        record_agent_response_at(&connection, "fixture-session-pine", newer_prompt)?;

        assert_eq!(
            agent_response_timestamps(sessions_dir.path())?,
            HashMap::from([
                ("fixture-session-maple".to_string(), latest),
                ("fixture-session-pine".to_string(), newer_prompt),
            ]),
        );
        Ok(())
    }
}
