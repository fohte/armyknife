use std::cell::Cell;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use rusqlite_migration::{M, Migrations};

use crate::commands::agent::error::CcError;
use crate::commands::agent::types::Session;

mod migration;
mod wake;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

static MIGRATION_ENTRIES: [M<'static>; 1] = [M::up(
    "CREATE TABLE sessions (
            session_id TEXT PRIMARY KEY,
            data TEXT NOT NULL,
            rev INTEGER NOT NULL
        );
        CREATE TABLE session_store_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        INSERT INTO session_store_meta (key, value) VALUES ('revision', '0');",
)];

static MIGRATIONS: LazyLock<Migrations<'static>> =
    LazyLock::new(|| Migrations::from_slice(&MIGRATION_ENTRIES));

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
    MIGRATIONS
        .to_latest(&mut connection)
        .context("migrating session database")?;

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
    match read_stored_session(&connection, session_id)? {
        StoredSession::Missing | StoredSession::Undecodable => Ok(None),
        StoredSession::Decoded(session) => Ok(Some(*session)),
    }
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
        read_session(&self.connection, &self.session_id)
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
    let Some(mut session) = (match read_stored_session(&transaction, session_id)? {
        StoredSession::Missing | StoredSession::Undecodable => None,
        StoredSession::Decoded(session) => Some(*session),
    }) else {
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

fn read_session(connection: &Connection, session_id: &str) -> Result<Option<Session>> {
    match read_stored_session(connection, session_id)? {
        StoredSession::Missing => Ok(None),
        StoredSession::Decoded(session) => Ok(Some(*session)),
        StoredSession::Undecodable => Err(anyhow!(
            "session {session_id} is stored but its data cannot be decoded"
        )),
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
