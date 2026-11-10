use std::path::Path;

use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{increment_revision, open_connection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchWebSessionState {
    pub(crate) session_id: String,
    pub(crate) later_until: Option<DateTime<Utc>>,
    pub(crate) activity_snapshot: Option<DateTime<Utc>>,
    pub(crate) until_activity: bool,
    pub(crate) is_today: bool,
}

pub(in crate::commands::agent::store) fn watch_web_session_states(
    sessions_dir: &Path,
) -> Result<Vec<WatchWebSessionState>> {
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }
    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare(
        "SELECT session_id, later_until, activity_snapshot, until_activity, is_today
         FROM watch_web_session_state",
    )?;
    let rows = statement.query_map([], |row| {
        let later_until: Option<String> = row.get(1)?;
        let activity_snapshot: Option<String> = row.get(2)?;
        Ok((
            row.get::<_, String>(0)?,
            later_until,
            activity_snapshot,
            row.get::<_, bool>(3)?,
            row.get::<_, bool>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (session_id, later_until, activity_snapshot, until_activity, is_today) = row?;
        Ok(WatchWebSessionState {
            session_id,
            later_until: parse_timestamp(later_until)?,
            activity_snapshot: parse_timestamp(activity_snapshot)?,
            until_activity,
            is_today,
        })
    })
    .collect()
}

pub(in crate::commands::agent::store) fn set_watch_web_later(
    sessions_dir: &Path,
    session_id: &str,
    later_until: Option<DateTime<Utc>>,
    activity_snapshot: DateTime<Utc>,
    until_activity: bool,
) -> Result<bool> {
    update_session_state(sessions_dir, session_id, |transaction| {
        transaction.execute(
            "INSERT INTO watch_web_session_state
                (session_id, later_until, activity_snapshot, until_activity)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET
                later_until = excluded.later_until,
                activity_snapshot = excluded.activity_snapshot,
                until_activity = excluded.until_activity",
            params![
                session_id,
                later_until.map(|value| value.to_rfc3339()),
                activity_snapshot.to_rfc3339(),
                until_activity,
            ],
        )?;
        Ok(())
    })
}

pub(in crate::commands::agent::store) fn clear_watch_web_later(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<bool> {
    update_session_state(sessions_dir, session_id, |transaction| {
        transaction.execute(
            "INSERT INTO watch_web_session_state
                (session_id, later_until, activity_snapshot, until_activity)
             VALUES (?1, NULL, NULL, 0)
             ON CONFLICT(session_id) DO UPDATE SET
                later_until = NULL,
                activity_snapshot = NULL,
                until_activity = 0",
            [session_id],
        )?;
        Ok(())
    })
}

pub(in crate::commands::agent::store) fn set_watch_web_today(
    sessions_dir: &Path,
    session_id: &str,
    is_today: bool,
) -> Result<bool> {
    update_session_state(sessions_dir, session_id, |transaction| {
        transaction.execute(
            "INSERT INTO watch_web_session_state (session_id, is_today)
             VALUES (?1, ?2)
             ON CONFLICT(session_id) DO UPDATE SET is_today = excluded.is_today",
            params![session_id, is_today],
        )?;
        Ok(())
    })
}

pub(in crate::commands::agent::store) fn watch_web_user_marker(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<Option<String>> {
    super::validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(None);
    }
    let connection = open_connection(sessions_dir)?;
    connection
        .query_row(
            "SELECT watch_web_user_marker FROM watch_web_session_state WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(Into::into)
}

pub(in crate::commands::agent::store) fn ensure_watch_web_user_marker(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<String> {
    super::validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let session_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
        [session_id],
        |row| row.get(0),
    )?;
    let marker: Option<String> = transaction
        .query_row(
            "SELECT watch_web_user_marker FROM watch_web_session_state WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    if let Some(marker) = marker {
        transaction.commit()?;
        return Ok(marker);
    }

    let marker = crate::commands::agent::watch_web_user_marker::generate();
    transaction.execute(
        "INSERT INTO watch_web_session_state (session_id, watch_web_user_marker)
         VALUES (?1, ?2)
         ON CONFLICT(session_id) DO UPDATE SET watch_web_user_marker = excluded.watch_web_user_marker",
        params![session_id, marker],
    )?;
    if session_exists {
        let revision = increment_revision(&transaction)?;
        transaction.execute(
            "UPDATE sessions SET rev = ?1 WHERE session_id = ?2",
            params![revision, session_id],
        )?;
    }
    transaction.commit()?;
    Ok(marker)
}

fn update_session_state(
    sessions_dir: &Path,
    session_id: &str,
    update_state: impl FnOnce(&Connection) -> Result<()>,
) -> Result<bool> {
    super::validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(false);
    }

    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
        [session_id],
        |row| row.get(0),
    )?;
    if !exists {
        transaction.rollback()?;
        return Ok(false);
    }

    update_state(&transaction)?;
    let revision = increment_revision(&transaction)?;
    let changed = transaction.execute(
        "UPDATE sessions SET rev = ?1 WHERE session_id = ?2",
        params![revision, session_id],
    )?;
    ensure!(
        changed == 1,
        "session {session_id} disappeared during state update"
    );
    transaction.commit()?;
    Ok(true)
}

fn parse_timestamp(value: Option<String>) -> Result<Option<DateTime<Utc>>> {
    value
        .map(|value| DateTime::parse_from_rfc3339(&value).map(|value| value.with_timezone(&Utc)))
        .transpose()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::test_support::{session, temp_dir};
    use super::*;
    use crate::commands::agent::store::database::{save_session, session_store_revision};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    #[fixture]
    fn saved_session_dir(
        temp_dir: TempDir,
        session: crate::commands::agent::types::Session,
    ) -> TempDir {
        save_session(temp_dir.path(), &session).expect("session should save");
        temp_dir
    }

    #[rstest]
    fn updating_later_state_and_revision_is_atomic(
        saved_session_dir: TempDir,
        session: crate::commands::agent::types::Session,
    ) {
        let before_revision =
            session_store_revision(saved_session_dir.path()).expect("revision should load");

        let changed = set_watch_web_later(
            saved_session_dir.path(),
            &session.session_id,
            None,
            session.updated_at,
            true,
        )
        .expect("state should update");

        assert_eq!(
            (
                changed,
                session_store_revision(saved_session_dir.path()).expect("revision should load"),
                watch_web_session_states(saved_session_dir.path()).expect("state should load"),
            ),
            (
                true,
                before_revision + 1,
                vec![WatchWebSessionState {
                    session_id: session.session_id,
                    later_until: None,
                    activity_snapshot: Some(session.updated_at),
                    until_activity: true,
                    is_today: false,
                }],
            ),
        );
    }

    #[rstest]
    fn state_update_does_not_create_orphan_rows(temp_dir: TempDir) {
        let changed = set_watch_web_today(temp_dir.path(), "missing-session", true)
            .expect("missing session should be ignored");

        assert_eq!(
            (
                changed,
                watch_web_session_states(temp_dir.path()).expect("state should load")
            ),
            (false, Vec::new()),
        );
    }

    #[rstest]
    fn user_markers_are_stable_and_unique_per_session(
        saved_session_dir: TempDir,
        session: crate::commands::agent::types::Session,
    ) {
        let first = ensure_watch_web_user_marker(saved_session_dir.path(), &session.session_id)
            .expect("marker should be created");
        let repeated = ensure_watch_web_user_marker(saved_session_dir.path(), &session.session_id)
            .expect("marker should be reused");
        let mut other_session = session.clone();
        other_session.session_id = "other-state-session".to_string();
        save_session(saved_session_dir.path(), &other_session).expect("second session should save");
        let other =
            ensure_watch_web_user_marker(saved_session_dir.path(), &other_session.session_id)
                .expect("second marker should be created");

        assert_eq!(
            (
                repeated,
                watch_web_user_marker(saved_session_dir.path(), &session.session_id)
                    .expect("first marker should load"),
                first != other,
                watch_web_user_marker(saved_session_dir.path(), &other_session.session_id)
                    .expect("second marker should load"),
            ),
            (first.clone(), Some(first), true, Some(other)),
        );
    }
}

#[cfg(test)]
pub(super) mod test_support {
    use crate::commands::agent::types::{Engine, Session, SessionStatus};
    use chrono::{TimeZone, Utc};
    use rstest::fixture;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[fixture]
    pub(super) fn temp_dir() -> TempDir {
        TempDir::new().expect("temporary directory should be created")
    }

    #[fixture]
    pub(super) fn session() -> Session {
        Session {
            session_id: "state-session".to_string(),
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/workspace/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: Utc.timestamp_opt(1_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_000, 0).unwrap(),
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            launched_with_vibe: false,
            work_type_pinned: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }
}
