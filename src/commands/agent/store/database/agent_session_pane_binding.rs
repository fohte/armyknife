use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use super::{open_connection, validate_session_id};

// Startup can save a classification before a session row exists.
const ORPHANED_CLASSIFICATION_TTL_MILLIS: i64 = 24 * 60 * 60 * 1000;

pub(super) fn has_parent_agent_process_from_connection(
    connection: &Connection,
    session_id: &str,
) -> Result<Option<bool>> {
    let now = chrono::Utc::now().timestamp_millis();
    let state = connection
        .query_row(
            "SELECT has_parent_agent FROM agent_session_pane_binding_state
             WHERE session_id = ?1
               AND (
                   created_at >= ?2
                   OR EXISTS (
                       SELECT 1 FROM sessions
                       WHERE sessions.session_id = agent_session_pane_binding_state.session_id
                   )
               )",
            params![session_id, now - ORPHANED_CLASSIFICATION_TTL_MILLIS],
            |row| row.get::<_, bool>(0),
        )
        .optional()?;
    Ok(state)
}

pub(crate) fn set_has_parent_agent_process(
    sessions_dir: &Path,
    session_id: &str,
    has_parent_agent: Option<bool>,
) -> Result<()> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let now = chrono::Utc::now().timestamp_millis();
    transaction.execute(
        "DELETE FROM agent_session_pane_binding_state
         WHERE created_at < ?1
           AND NOT EXISTS (
               SELECT 1 FROM sessions
               WHERE sessions.session_id = agent_session_pane_binding_state.session_id
           )",
        [now - ORPHANED_CLASSIFICATION_TTL_MILLIS],
    )?;
    match has_parent_agent {
        Some(has_parent_agent) => {
            transaction.execute(
                "INSERT INTO agent_session_pane_binding_state
                    (session_id, has_parent_agent, created_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(session_id) DO UPDATE SET
                    has_parent_agent = excluded.has_parent_agent,
                    created_at = excluded.created_at",
                params![session_id, has_parent_agent, now],
            )?;
        }
        None => {
            transaction.execute(
                "DELETE FROM agent_session_pane_binding_state WHERE session_id = ?1",
                [session_id],
            )?;
        }
    }
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::{fixture, rstest};

    use super::*;

    #[fixture]
    fn sessions_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temporary sessions directory should be created")
    }

    #[fixture]
    fn stale_orphaned_classification(sessions_dir: tempfile::TempDir) -> tempfile::TempDir {
        let connection =
            open_connection(sessions_dir.path()).expect("session database should be opened");
        let created_at =
            chrono::Utc::now().timestamp_millis() - ORPHANED_CLASSIFICATION_TTL_MILLIS - 1;
        connection
            .execute(
                indoc! {"
                    INSERT INTO agent_session_pane_binding_state
                    (session_id, has_parent_agent, created_at)
                    VALUES (?1, ?2, ?3)
                "},
                params!["fixture-session-orphan", true, created_at],
            )
            .expect("orphaned classification should be inserted");
        sessions_dir
    }

    #[rstest]
    #[case::direct_agent(false)]
    #[case::nested_agent(true)]
    fn parent_agent_state_persists_per_session(
        sessions_dir: tempfile::TempDir,
        #[case] state: bool,
    ) -> Result<()> {
        set_has_parent_agent_process(sessions_dir.path(), "fixture-session-cedar", Some(state))?;
        let connection = open_connection(sessions_dir.path())?;
        let state_exists = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_session_pane_binding_state WHERE session_id = ?1)",
            ["fixture-session-cedar"],
            |row| row.get::<_, bool>(0),
        )?;

        assert_eq!(
            (
                has_parent_agent_process_from_connection(&connection, "fixture-session-cedar",)?,
                state_exists,
            ),
            (Some(state), true),
        );
        Ok(())
    }

    #[rstest]
    fn missing_parent_agent_state_is_unavailable(sessions_dir: tempfile::TempDir) -> Result<()> {
        let connection = open_connection(sessions_dir.path())?;
        let state_exists = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_session_pane_binding_state WHERE session_id = ?1)",
            ["fixture-session-cedar"],
            |row| row.get::<_, bool>(0),
        )?;
        assert_eq!(
            (
                has_parent_agent_process_from_connection(&connection, "fixture-session-cedar",)?,
                state_exists,
            ),
            (None, false),
        );
        Ok(())
    }

    #[rstest]
    fn stale_orphaned_parent_agent_state_is_not_loaded(
        stale_orphaned_classification: tempfile::TempDir,
    ) -> Result<()> {
        let connection = open_connection(stale_orphaned_classification.path())?;

        assert_eq!(
            has_parent_agent_process_from_connection(&connection, "fixture-session-orphan",)?,
            None,
        );
        Ok(())
    }

    #[rstest]
    fn saving_classification_expires_stale_orphans_and_preserves_existing_sessions(
        stale_orphaned_classification: tempfile::TempDir,
    ) -> Result<()> {
        let connection = open_connection(stale_orphaned_classification.path())?;
        let created_at =
            chrono::Utc::now().timestamp_millis() - ORPHANED_CLASSIFICATION_TTL_MILLIS - 1;
        connection.execute(
            "INSERT INTO sessions (session_id, data, rev) VALUES (?1, ?2, ?3)",
            params!["fixture-session-active", "{}", 0],
        )?;
        connection.execute(
            indoc! {"
                INSERT INTO agent_session_pane_binding_state
                (session_id, has_parent_agent, created_at)
                VALUES (?1, ?2, ?3)
            "},
            params!["fixture-session-active", true, created_at],
        )?;

        set_has_parent_agent_process(
            stale_orphaned_classification.path(),
            "fixture-session-current",
            Some(false),
        )?;
        let connection = open_connection(stale_orphaned_classification.path())?;
        let mut statement = connection.prepare(indoc! {"
                SELECT session_id, has_parent_agent FROM agent_session_pane_binding_state
                ORDER BY session_id
            "})?;
        let states = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        assert_eq!(
            states,
            vec![
                ("fixture-session-active".to_string(), true),
                ("fixture-session-current".to_string(), false),
            ],
        );
        Ok(())
    }

    #[rstest]
    fn parent_agent_state_is_replaced_after_a_later_session_start(
        sessions_dir: tempfile::TempDir,
    ) -> Result<()> {
        set_has_parent_agent_process(sessions_dir.path(), "fixture-session-cedar", Some(false))?;
        set_has_parent_agent_process(sessions_dir.path(), "fixture-session-cedar", Some(true))?;
        let connection = open_connection(sessions_dir.path())?;

        assert_eq!(
            has_parent_agent_process_from_connection(&connection, "fixture-session-cedar",)?,
            Some(true),
        );
        Ok(())
    }

    #[rstest]
    fn clearing_session_start_classification_removes_the_cached_state(
        sessions_dir: tempfile::TempDir,
    ) -> Result<()> {
        set_has_parent_agent_process(sessions_dir.path(), "fixture-session-cedar", Some(false))?;
        set_has_parent_agent_process(sessions_dir.path(), "fixture-session-cedar", None)?;
        let connection = open_connection(sessions_dir.path())?;
        let state_exists = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_session_pane_binding_state WHERE session_id = ?1)",
            ["fixture-session-cedar"],
            |row| row.get::<_, bool>(0),
        )?;

        assert_eq!(
            (
                has_parent_agent_process_from_connection(&connection, "fixture-session-cedar",)?,
                state_exists,
            ),
            (None, false),
        );
        Ok(())
    }
}
