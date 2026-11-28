use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use super::{open_connection, validate_session_id};

pub(super) fn has_parent_agent_process_from_connection(
    connection: &Connection,
    session_id: &str,
) -> Result<Option<bool>> {
    let state = connection
        .query_row(
            "SELECT has_parent_agent FROM agent_session_pane_binding_state
             WHERE session_id = ?1",
            [session_id],
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
    let connection = open_connection(sessions_dir)?;
    match has_parent_agent {
        Some(has_parent_agent) => {
            connection.execute(
                "INSERT INTO agent_session_pane_binding_state (session_id, has_parent_agent)
                 VALUES (?1, ?2)
                 ON CONFLICT(session_id) DO UPDATE SET has_parent_agent = excluded.has_parent_agent",
                params![session_id, has_parent_agent],
            )?;
        }
        None => {
            connection.execute(
                "DELETE FROM agent_session_pane_binding_state WHERE session_id = ?1",
                [session_id],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};

    use super::*;

    #[fixture]
    fn sessions_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temporary sessions directory should be created")
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
