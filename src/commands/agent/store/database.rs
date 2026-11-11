use std::cell::Cell;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::commands::agent::error::CcError;
use crate::commands::agent::types::Session;

mod decisions;
mod migration;
mod migrations;
mod peer_out_messages;
mod proposals;
mod queued_prompts;
mod wake;
mod web_state;

pub(crate) use decisions::{
    AgentDecision, AgentDecisionCounts, agent_decision_counts, agent_decisions,
    insert_agent_decision, set_agent_decision_review,
};
pub(crate) use peer_out_messages::WatchWebPeerOutMessage;
pub(crate) use peer_out_messages::{
    insert_watch_web_peer_out_message, watch_web_peer_out_messages,
};
pub(crate) use proposals::AgentProposal;
pub(crate) use proposals::{
    add_agent_proposal, agent_proposals, close_unanswered_agent_proposals,
    unanswered_agent_proposal_session_ids, write_agent_proposal_answer,
};
pub(crate) use queued_prompts::WatchWebQueuedPrompt;
pub(crate) use queued_prompts::{
    delete_watch_web_queued_prompts, insert_watch_web_queued_prompt, watch_web_queued_prompts,
};
pub(crate) use web_state::WatchWebSessionState;
pub(crate) use web_state::set_watch_web_compacting;
pub(super) use web_state::{
    clear_watch_web_later, set_watch_web_later, set_watch_web_today, watch_web_session_states,
    watch_web_user_marker,
};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(crate) struct SessionStoreChanges {
    pub(crate) changed_sessions: Vec<Session>,
    pub(crate) existing_session_ids: HashSet<String>,
    pub(crate) revision: i64,
}

/// Holds the database write transaction across a session read-modify-write.
/// Saving commits before tmux calls can hold the database-wide writer lock.
pub(crate) struct SessionLock {
    connection: Connection,
    session_id: String,
    transaction_active: Cell<bool>,
}

pub(crate) struct WakeLock {
    _guard: wake::WakeLock,
}

pub(super) enum StoredSession {
    Missing,
    Decoded(Box<Session>),
    Undecodable,
}

pub(crate) fn open_connection(sessions_dir: &Path) -> Result<Connection> {
    fs::create_dir_all(sessions_dir)
        .with_context(|| format!("creating sessions dir {}", sessions_dir.display()))?;

    let database_path = sessions_dir.join("sessions.db");
    let mut connection = Connection::open(&database_path)
        .with_context(|| format!("opening session database {}", database_path.display()))?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    migrations::migrate_if_needed(&mut connection)?;

    if let Err(error) = migration::import_legacy_json(&mut connection, sessions_dir) {
        tracing::warn!(
            target: "armyknife::commands::agent::store",
            event = "agent.store.legacy_import_failed",
            error = %error,
        );
    }
    migration::remove_legacy_lock_files(sessions_dir);
    migration::cleanup_expired_legacy_json(&connection, sessions_dir)?;

    Ok(connection)
}

pub(crate) fn load_session(sessions_dir: &Path, session_id: &str) -> Result<Option<Session>> {
    validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(None);
    }
    let connection = open_connection(sessions_dir)?;
    read_session_skipping_undecodable(&connection, session_id)
}

pub(crate) fn save_session(sessions_dir: &Path, session: &Session) -> Result<()> {
    validate_session_id(&session.session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    write_session(&transaction, session)?;
    transaction.commit()?;
    Ok(())
}

pub(crate) fn lock_session_for_update(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<SessionLock> {
    validate_session_id(session_id)?;
    let connection = open_connection(sessions_dir)?;
    connection.execute_batch("BEGIN IMMEDIATE")?;

    Ok(SessionLock {
        connection,
        session_id: session_id.to_string(),
        transaction_active: Cell::new(true),
    })
}

pub(crate) fn lock_session_for_wake(sessions_dir: &Path, session_id: &str) -> Result<WakeLock> {
    validate_session_id(session_id)?;
    Ok(WakeLock {
        _guard: wake::acquire(sessions_dir, session_id)?,
    })
}

impl SessionLock {
    pub(crate) fn load(&self) -> Result<Option<Session>> {
        read_session_strict(&self.connection, &self.session_id)
    }

    pub(crate) fn ensure_watch_web_user_marker(&self) -> Result<String> {
        self.ensure_transaction()?;
        let marker = web_state::ensure_watch_web_user_marker_in_transaction(
            &self.connection,
            &self.session_id,
        )?;
        self.connection.execute_batch("COMMIT")?;
        self.transaction_active.set(false);
        Ok(marker)
    }

    pub(crate) fn save(&self, session: &Session) -> Result<()> {
        ensure!(
            session.session_id == self.session_id,
            "session ID cannot change while its store lock is held"
        );
        self.ensure_transaction()?;
        write_session(&self.connection, session)?;
        self.connection.execute_batch("COMMIT")?;
        self.transaction_active.set(false);
        Ok(())
    }

    pub(crate) fn delete(&self) -> Result<bool> {
        self.ensure_transaction()?;
        let removed = delete_session_row(&self.connection, &self.session_id)?;
        self.connection.execute_batch("COMMIT")?;
        self.transaction_active.set(false);
        Ok(removed)
    }

    fn ensure_transaction(&self) -> Result<()> {
        if !self.transaction_active.get() {
            self.connection.execute_batch("BEGIN IMMEDIATE")?;
            self.transaction_active.set(true);
        }
        Ok(())
    }
}

impl Drop for SessionLock {
    fn drop(&mut self) {
        if self.transaction_active.get() {
            let _ = self.connection.execute_batch("ROLLBACK");
        }
    }
}

pub(crate) fn update_session(
    sessions_dir: &Path,
    session_id: &str,
    mutate: impl FnOnce(&mut Session) -> bool,
) -> Result<()> {
    validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(());
    }
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(mut session) = read_session_skipping_undecodable(&transaction, session_id)? else {
        transaction.commit()?;
        return Ok(());
    };
    if mutate(&mut session) {
        ensure!(
            session.session_id == session_id,
            "session ID cannot change during an update"
        );
        write_session(&transaction, &session)?;
    }
    transaction.commit()?;
    Ok(())
}

pub(crate) fn touch_session(sessions_dir: &Path, session_id: &str) -> Result<()> {
    validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(());
    }
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
        [session_id],
        |row| row.get(0),
    )?;
    if exists {
        let revision = increment_revision(&transaction)?;
        transaction.execute(
            "UPDATE sessions SET rev = ?1 WHERE session_id = ?2",
            params![revision, session_id],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

pub(crate) fn delete_session(sessions_dir: &Path, session_id: &str) -> Result<bool> {
    validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(false);
    }
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let removed = delete_session_row(&transaction, session_id)?;
    transaction.commit()?;
    Ok(removed)
}

pub(crate) fn all_sessions(sessions_dir: &Path) -> Result<Vec<Session>> {
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }
    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare("SELECT session_id, data FROM sessions")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut sessions = Vec::new();
    for row in rows {
        let (session_id, data) = row?;
        if let Some(session) = decode_session(&session_id, &data) {
            sessions.push(session);
        }
    }
    Ok(sessions)
}

pub(crate) fn session_store_revision(sessions_dir: &Path) -> Result<i64> {
    let connection = open_connection(sessions_dir)?;
    read_revision(&connection)
}

pub(crate) fn session_changes_since(
    sessions_dir: &Path,
    revision: i64,
) -> Result<SessionStoreChanges> {
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let changes = {
        let mut statement = transaction.prepare(
            "SELECT session_id, data FROM sessions WHERE rev > ?1 ORDER BY rev, session_id",
        )?;
        let rows = statement.query_map([revision], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut changed_sessions = Vec::new();
        for row in rows {
            let (session_id, data) = row?;
            if let Some(session) = decode_session(&session_id, &data) {
                changed_sessions.push(session);
            }
        }

        let mut ids_statement = transaction.prepare("SELECT session_id FROM sessions")?;
        let existing_session_ids = ids_statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<HashSet<String>>>()?;

        SessionStoreChanges {
            changed_sessions,
            existing_session_ids,
            revision: read_revision(&transaction)?,
        }
    };
    transaction.commit()?;
    Ok(changes)
}

fn read_session_strict(connection: &Connection, session_id: &str) -> Result<Option<Session>> {
    match read_stored_session(connection, session_id)? {
        StoredSession::Missing => Ok(None),
        StoredSession::Decoded(session) => Ok(Some(*session)),
        StoredSession::Undecodable => Err(anyhow!(
            "session {session_id} is stored but its data cannot be decoded"
        )),
    }
}

fn read_session_skipping_undecodable(
    connection: &Connection,
    session_id: &str,
) -> Result<Option<Session>> {
    match read_stored_session(connection, session_id)? {
        StoredSession::Missing | StoredSession::Undecodable => Ok(None),
        StoredSession::Decoded(session) => Ok(Some(*session)),
    }
}

pub(super) fn read_stored_session(
    connection: &Connection,
    session_id: &str,
) -> Result<StoredSession> {
    let data = connection
        .query_row(
            "SELECT data FROM sessions WHERE session_id = ?1",
            [session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(data) = data else {
        return Ok(StoredSession::Missing);
    };
    Ok(match decode_session(session_id, &data) {
        Some(session) => StoredSession::Decoded(Box::new(session)),
        None => StoredSession::Undecodable,
    })
}

fn decode_session(session_id: &str, data: &str) -> Option<Session> {
    match serde_json::from_str(data) {
        Ok(session) => Some(session),
        Err(error) => {
            tracing::warn!(
                target: "armyknife::commands::agent::store",
                event = "agent.store.session_decode_failed",
                session = session_id,
                error = %error,
            );
            None
        }
    }
}

pub(crate) fn serialize_session(session: &Session) -> Result<String> {
    let mut persisted = session.clone();
    persisted
        .pending_bg_task_ids
        .remove(crate::commands::agent::types::BG_RUN_PENDING_TASK_MARKER);
    Ok(serde_json::to_string(&persisted)?)
}

pub(crate) fn write_session(connection: &Connection, session: &Session) -> Result<i64> {
    ensure!(
        !matches!(
            read_stored_session(connection, &session.session_id)?,
            StoredSession::Undecodable
        ),
        "refusing to overwrite undecodable session {}",
        session.session_id
    );
    let data = serialize_session(session)?;
    let revision = increment_revision(connection)?;
    connection.execute(
        "INSERT INTO sessions (session_id, data, rev) VALUES (?1, ?2, ?3)
         ON CONFLICT(session_id) DO UPDATE SET data = excluded.data, rev = excluded.rev",
        params![session.session_id, data, revision],
    )?;
    Ok(revision)
}

pub(crate) fn increment_revision(connection: &Connection) -> Result<i64> {
    connection.execute(
        "UPDATE session_store_meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'revision'",
        [],
    )?;
    read_revision(connection)
}

fn read_revision(connection: &Connection) -> Result<i64> {
    Ok(connection.query_row(
        "SELECT CAST(value AS INTEGER) FROM session_store_meta WHERE key = 'revision'",
        [],
        |row| row.get(0),
    )?)
}

fn delete_session_row(connection: &Connection, session_id: &str) -> Result<bool> {
    let changed = connection.execute("DELETE FROM sessions WHERE session_id = ?1", [session_id])?;
    if changed > 0 {
        connection.execute(
            "DELETE FROM watch_web_session_state WHERE session_id = ?1",
            [session_id],
        )?;
        connection.execute(
            "DELETE FROM watch_web_queued_prompt WHERE session_id = ?1",
            [session_id],
        )?;
        connection.execute(
            "DELETE FROM watch_web_peer_out_message WHERE from_session_id = ?1",
            [session_id],
        )?;
        connection.execute(
            "DELETE FROM agent_decision WHERE session_id = ?1",
            [session_id],
        )?;
        increment_revision(connection)?;
    }
    Ok(changed > 0)
}

pub(crate) fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.is_empty()
        || session_id.contains('/')
        || session_id.contains('\\')
        || session_id.contains("..")
    {
        return Err(CcError::InvalidSessionId(session_id.to_string()).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::Value;

    #[test]
    fn adds_watch_web_state_without_changing_existing_session_rows() {
        let temp = tempfile::tempdir().unwrap();
        let database_path = temp.path().join("sessions.db");
        let connection = Connection::open(&database_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sessions (
                    session_id TEXT PRIMARY KEY,
                    data TEXT NOT NULL,
                    rev INTEGER NOT NULL
                );
                CREATE TABLE session_store_meta (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                INSERT INTO session_store_meta (key, value) VALUES ('revision', '7');
                INSERT INTO sessions (session_id, data, rev)
                    VALUES ('legacy-session', 'preserved-data', 7);
                PRAGMA user_version = 1;",
            )
            .unwrap();
        drop(connection);

        let connection = open_connection(temp.path()).unwrap();
        let preserved: (String, i64, i64) = connection
            .query_row(
                "SELECT data, rev, (SELECT CAST(value AS INTEGER)
                 FROM session_store_meta WHERE key = 'revision')
                 FROM sessions WHERE session_id = 'legacy-session'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let state_table_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'watch_web_session_state')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let queued_prompt_table_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'watch_web_queued_prompt')",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(
            (preserved, state_table_exists, queued_prompt_table_exists),
            (("preserved-data".to_string(), 7, 7), true, true),
        );
    }

    #[rstest]
    #[case::newer_database(true, migrations::current_version() + 1)]
    #[case::unmigrated_database(false, migrations::current_version())]
    fn opens_database_and_reads_and_writes_sessions(
        #[case] has_newer_schema: bool,
        #[case] expected_user_version: i64,
    ) {
        let temp_dir = tempfile::tempdir().expect("temporary directory should be created");
        let sessions_dir = temp_dir.path().join("sessions");
        fs::create_dir_all(&sessions_dir).expect("sessions directory should be created");

        if has_newer_schema {
            let connection = open_connection(&sessions_dir)
                .expect("current migrations should create the database");
            drop(connection);
            let database_path = sessions_dir.join("sessions.db");
            let connection =
                Connection::open(database_path).expect("temporary session database should open");
            connection
                .execute_batch("CREATE TABLE future_data (value TEXT);")
                .expect("future schema table should be created");
            connection
                .pragma_update(None, "user_version", expected_user_version)
                .expect("future migration version should be stored");
        }

        let expected: Session = serde_json::from_value(serde_json::json!({
            "session_id": "sample-session",
            "cwd": "/sample/worktree",
            "transcript_path": null,
            "tmux_info": null,
            "status": "stopped",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .expect("test session should deserialize");
        super::super::save_session_to(&sessions_dir, &expected)
            .expect("session should save to the temporary database");
        let actual = super::super::load_session_from(&sessions_dir, &expected.session_id)
            .expect("session should load from the temporary database")
            .expect("saved session should exist");
        let actual: Value = serde_json::to_value(actual).expect("loaded session should serialize");
        let expected: Value =
            serde_json::to_value(expected).expect("expected session should serialize");
        let connection = Connection::open(sessions_dir.join("sessions.db"))
            .expect("temporary session database should open");
        let actual_user_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("database migration version should be readable");

        assert_eq!(
            (actual, actual_user_version),
            (expected, expected_user_version),
        );
    }
}
