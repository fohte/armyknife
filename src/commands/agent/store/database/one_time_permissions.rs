use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::Value;

use super::{open_connection, validate_session_id};

pub(crate) fn insert_permission_retry_approval(
    sessions_dir: &Path,
    session_id: &str,
    tool_name: &str,
    tool_input: &Value,
    expires_at: i64,
) -> Result<i64> {
    validate_session_id(session_id)?;
    let input = serde_json::to_string(tool_input).context("serializing approved tool input")?;
    let connection = open_connection(sessions_dir)?;
    connection.execute(
        "INSERT INTO agent_permission_retry_approval
            (session_id, tool_name, tool_input, expires_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![session_id, tool_name, input, expires_at],
    )?;
    Ok(connection.last_insert_rowid())
}

pub(crate) fn revoke_permission_retry_approval(
    sessions_dir: &Path,
    approval_id: i64,
) -> Result<bool> {
    let connection = open_connection(sessions_dir)?;
    let changed = connection.execute(
        "DELETE FROM agent_permission_retry_approval WHERE id = ?1",
        [approval_id],
    )?;
    Ok(changed > 0)
}

pub(crate) fn consume_permission_retry_approval(
    sessions_dir: &Path,
    session_id: &str,
    tool_name: &str,
    tool_input: &Value,
    now: i64,
) -> Result<bool> {
    validate_session_id(session_id)?;
    let input = serde_json::to_string(tool_input).context("serializing attempted tool input")?;
    let mut connection = open_connection(sessions_dir)?;
    let approval_id = connection
        .query_row(
            "SELECT id FROM agent_permission_retry_approval
             WHERE session_id = ?1 AND tool_name = ?2 AND tool_input = ?3
               AND expires_at > ?4
             ORDER BY id LIMIT 1",
            params![session_id, tool_name, input, now],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(approval_id) = approval_id else {
        return Ok(false);
    };

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let consumed = transaction.execute(
        "DELETE FROM agent_permission_retry_approval
         WHERE id = ?1 AND session_id = ?2 AND tool_name = ?3
           AND tool_input = ?4 AND expires_at > ?5",
        params![approval_id, session_id, tool_name, input, now],
    )?;
    transaction.commit()?;
    Ok(consumed > 0)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rstest::{fixture, rstest};
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[fixture]
    fn sessions_dir() -> TempDir {
        TempDir::new().expect("temporary sessions directory should be created")
    }

    fn save_approval(sessions_dir: &Path, expires_at: i64) {
        insert_permission_retry_approval(
            sessions_dir,
            "fixture-session-cedar",
            "Bash",
            &json!({ "command": "printf fixture" }),
            expires_at,
        )
        .expect("approval should be saved");
    }

    #[test]
    fn exact_approval_is_consumed_once() {
        let sessions_dir = TempDir::new().expect("temporary sessions directory should be created");
        save_approval(sessions_dir.path(), 1200);

        assert_eq!(
            (
                consume_permission_retry_approval(
                    sessions_dir.path(),
                    "fixture-session-cedar",
                    "Bash",
                    &json!({ "command": "printf fixture" }),
                    1000,
                )
                .expect("matching approval should be consumed"),
                consume_permission_retry_approval(
                    sessions_dir.path(),
                    "fixture-session-cedar",
                    "Bash",
                    &json!({ "command": "printf fixture" }),
                    1001,
                )
                .expect("second matching attempt should be checked"),
            ),
            (true, false),
        );
    }

    #[rstest]
    #[case::other_session("fixture-session-maple", "Bash", json!({ "command": "printf fixture" }), 1000)]
    #[case::other_tool("fixture-session-cedar", "Read", json!({ "command": "printf fixture" }), 1000)]
    #[case::different_input("fixture-session-cedar", "Bash", json!({ "command": "printf changed" }), 1000)]
    #[case::expired("fixture-session-cedar", "Bash", json!({ "command": "printf fixture" }), 1200)]
    fn nonmatching_or_expired_approval_is_not_consumed(
        sessions_dir: TempDir,
        #[case] session_id: &str,
        #[case] tool_name: &str,
        #[case] tool_input: Value,
        #[case] now: i64,
    ) {
        save_approval(sessions_dir.path(), 1200);

        assert!(
            !consume_permission_retry_approval(
                sessions_dir.path(),
                session_id,
                tool_name,
                &tool_input,
                now,
            )
            .expect("nonmatching approval should be rejected"),
        );
    }
}
