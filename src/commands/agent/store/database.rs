use std::cell::Cell;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use rusqlite_migration::{Error as MigrationError, M, MigrationDefinitionError, Migrations};

use crate::commands::agent::error::CcError;
use crate::commands::agent::types::Session;

mod decisions;
mod migration;
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

// Migrations must stay readable by older binaries: add tables or nullable columns only.
static MIGRATION_ENTRIES: [M<'static>; 8] = [
    M::up(
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
    ),
    M::up(
        "CREATE TABLE watch_web_session_state (
        session_id TEXT PRIMARY KEY,
        later_until TEXT,
        activity_snapshot TEXT,
        until_activity INTEGER NOT NULL DEFAULT 0,
        is_today INTEGER NOT NULL DEFAULT 0
    );",
    ),
    M::up("ALTER TABLE watch_web_session_state ADD COLUMN watch_web_user_marker TEXT;"),
    M::up(
        "ALTER TABLE watch_web_session_state
         ADD COLUMN is_compacting INTEGER;",
    ),
    M::up(
        "CREATE TABLE watch_web_queued_prompt (
            id INTEGER PRIMARY KEY,
            session_id TEXT NOT NULL,
            content TEXT NOT NULL,
            sent_at TEXT NOT NULL
        );
        CREATE INDEX watch_web_queued_prompt_session_id_id
            ON watch_web_queued_prompt (session_id, id);",
    ),
    M::up(
        "CREATE TABLE agent_proposal (
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            body TEXT NOT NULL,
            created_at TEXT NOT NULL,
            answer TEXT,
            resolved_at TEXT,
            PRIMARY KEY (session_id, seq)
        );
        CREATE INDEX agent_proposal_unresolved_session_id
            ON agent_proposal (session_id) WHERE resolved_at IS NULL;
        CREATE TRIGGER agent_proposal_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM agent_proposal WHERE session_id = OLD.session_id;
            END;",
    ),
    M::up(
        "CREATE TABLE IF NOT EXISTS agent_proposal (
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            body TEXT NOT NULL,
            created_at TEXT NOT NULL,
            answer TEXT,
            resolved_at TEXT,
            PRIMARY KEY (session_id, seq)
        );
        CREATE INDEX IF NOT EXISTS agent_proposal_unresolved_session_id
            ON agent_proposal (session_id) WHERE resolved_at IS NULL;
        CREATE TRIGGER IF NOT EXISTS agent_proposal_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM agent_proposal WHERE session_id = OLD.session_id;
            END;
        CREATE TABLE IF NOT EXISTS watch_web_peer_out_message (
            id INTEGER PRIMARY KEY,
            from_session_id TEXT NOT NULL,
            to_session_id TEXT NOT NULL,
            content TEXT NOT NULL,
            sent_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS watch_web_peer_out_message_sender_target_time
            ON watch_web_peer_out_message (from_session_id, to_session_id, sent_at, id);",
    ),
    M::up(
        "CREATE TABLE IF NOT EXISTS agent_proposal (
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            body TEXT NOT NULL,
            created_at TEXT NOT NULL,
            answer TEXT,
            resolved_at TEXT,
            PRIMARY KEY (session_id, seq)
        );
        CREATE INDEX IF NOT EXISTS agent_proposal_unresolved_session_id
            ON agent_proposal (session_id) WHERE resolved_at IS NULL;
        CREATE TRIGGER IF NOT EXISTS agent_proposal_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM agent_proposal WHERE session_id = OLD.session_id;
            END;
        CREATE TABLE IF NOT EXISTS agent_decision (
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            title TEXT NOT NULL,
            why TEXT NOT NULL,
            considered TEXT NOT NULL,
            easy_to_change TEXT NOT NULL,
            created_at TEXT NOT NULL,
            review TEXT,
            reviewed_at TEXT,
            PRIMARY KEY (session_id, seq)
        );
        CREATE TABLE IF NOT EXISTS watch_web_peer_out_message (
            id INTEGER PRIMARY KEY,
            from_session_id TEXT NOT NULL,
            to_session_id TEXT NOT NULL,
            content TEXT NOT NULL,
            sent_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS watch_web_peer_out_message_sender_target_time
            ON watch_web_peer_out_message (from_session_id, to_session_id, sent_at, id);",
    ),
];

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
    let user_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if user_version < MIGRATION_ENTRIES.len() as i64
        && let Err(error) = MIGRATIONS.to_latest(&mut connection)
    {
        accept_migration_error(&connection, error)?;
    }

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

fn accept_migration_error(connection: &Connection, error: MigrationError) -> Result<()> {
    let database_too_far_ahead = matches!(
        &error,
        MigrationError::MigrationDefinition(MigrationDefinitionError::DatabaseTooFarAhead)
    );
    if database_too_far_ahead {
        let user_version: i64 =
            connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if user_version > MIGRATION_ENTRIES.len() as i64 {
            return Ok(());
        }
    }

    Err(error).context("migrating session database")
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
    #[case::peer_message_schema(
        "CREATE TABLE watch_web_peer_out_message (
            id INTEGER PRIMARY KEY,
            from_session_id TEXT NOT NULL,
            to_session_id TEXT NOT NULL,
            content TEXT NOT NULL,
            sent_at TEXT NOT NULL
        );"
    )]
    #[case::proposal_schema(
        "CREATE TABLE agent_proposal (
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            body TEXT NOT NULL,
            created_at TEXT NOT NULL,
            answer TEXT,
            resolved_at TEXT,
            PRIMARY KEY (session_id, seq)
        );"
    )]
    fn upgrades_either_previous_version_six_schema(#[case] previous_schema: &str) {
        let temp_dir = tempfile::tempdir().expect("temporary sessions directory should be created");
        let database_path = temp_dir.path().join("sessions.db");
        let mut connection =
            Connection::open(database_path).expect("temporary session database should open");
        Migrations::from_slice(&MIGRATION_ENTRIES[..5])
            .to_latest(&mut connection)
            .expect("common schema should be created");
        connection
            .execute_batch(previous_schema)
            .expect("previous version six table should be created");
        connection
            .pragma_update(None, "user_version", 6)
            .expect("previous schema version should be stored");
        drop(connection);

        let connection = open_connection(temp_dir.path()).unwrap();
        let (peer_messages_exist, proposals_exist, decisions_exist): (bool, bool, bool) = connection
            .query_row(
                "SELECT
                    EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'watch_web_peer_out_message'),
                    EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'agent_proposal'),
                    EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'agent_decision')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        assert_eq!(
            (peer_messages_exist, proposals_exist, decisions_exist),
            (true, true, true),
        );
    }

    #[rstest]
    #[case::decision_only_v6(true, 1, 0)]
    #[case::proposal_only_v6(false, 0, 1)]
    fn upgrades_either_version_six_feature_layout(
        #[case] has_decision_table: bool,
        #[case] expected_decisions: i64,
        #[case] expected_proposals: i64,
    ) {
        let temp_dir = tempfile::tempdir().expect("temporary directory should be created");
        let sessions_dir = temp_dir.path().join("sessions");
        open_connection(&sessions_dir).expect("current migrations should create the database");

        let database_path = sessions_dir.join("sessions.db");
        let connection =
            Connection::open(&database_path).expect("temporary session database should open");
        if has_decision_table {
            connection
                .execute(
                    "INSERT INTO agent_decision
                     (session_id, seq, title, why, considered, easy_to_change, created_at)
                     VALUES ('sample-session', 1, 'sample choice', 'sample reason',
                             '[\"sample alternative\"]', 'sample change',
                             '2026-01-01T00:00:00Z')",
                    [],
                )
                .expect("decision from the earlier layout should be inserted");
            connection
                .execute_batch("DROP TABLE agent_proposal; PRAGMA user_version = 6;")
                .expect("proposal table should be removed to model the earlier layout");
        } else {
            connection
                .execute(
                    "INSERT INTO agent_proposal (session_id, seq, body, created_at)
                     VALUES ('sample-session', 1, 'sample proposal', '2026-01-01T00:00:00Z')",
                    [],
                )
                .expect("proposal from the merged layout should be inserted");
            connection
                .execute_batch("DROP TABLE agent_decision; PRAGMA user_version = 6;")
                .expect("decision table should be removed to model the merged layout");
        }
        drop(connection);

        let connection = open_connection(&sessions_dir)
            .expect("version six database should migrate to the combined schema");
        let schema_and_rows: (bool, bool, bool, bool, i64, i64) = connection
            .query_row(
                "SELECT
                    EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'agent_decision'),
                    EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'agent_proposal'),
                    EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'agent_proposal_unresolved_session_id'),
                    EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = 'agent_proposal_delete_session'),
                    (SELECT COUNT(*) FROM agent_decision),
                    (SELECT COUNT(*) FROM agent_proposal)",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .expect("combined schema and retained rows should be queryable");
        let user_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("database migration version should be readable");

        assert_eq!(
            (schema_and_rows, user_version),
            (
                (
                    true,
                    true,
                    true,
                    true,
                    expected_decisions,
                    expected_proposals,
                ),
                MIGRATION_ENTRIES.len() as i64,
            ),
        );
    }

    #[rstest]
    #[case::newer_database(true, MIGRATION_ENTRIES.len() as i64 + 1)]
    #[case::unmigrated_database(false, MIGRATION_ENTRIES.len() as i64)]
    fn opens_database_and_reads_and_writes_sessions(
        #[case] has_newer_schema: bool,
        #[case] expected_user_version: i64,
    ) {
        let temp_dir = tempfile::tempdir().expect("temporary directory should be created");
        let sessions_dir = temp_dir.path().join("sessions");
        fs::create_dir_all(&sessions_dir).expect("sessions directory should be created");

        if has_newer_schema {
            let database_path = sessions_dir.join("sessions.db");
            let mut connection =
                Connection::open(database_path).expect("temporary session database should open");
            MIGRATIONS
                .to_latest(&mut connection)
                .expect("known migrations should create the base schema");
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

    #[rstest]
    #[case::database_advanced_after_check(true, MIGRATION_ENTRIES.len() as i64 + 1)]
    #[case::database_not_advanced(false, MIGRATION_ENTRIES.len() as i64 - 1)]
    fn accepts_only_a_migration_error_for_a_newer_database(
        #[case] has_newer_version: bool,
        #[case] user_version: i64,
    ) {
        let temp_dir = tempfile::tempdir().expect("temporary directory should be created");
        let database_path = temp_dir.path().join("sessions.db");
        let connection =
            Connection::open(database_path).expect("temporary session database should open");
        connection
            .pragma_update(None, "user_version", user_version)
            .expect("database migration version should be set");
        let error =
            MigrationError::MigrationDefinition(MigrationDefinitionError::DatabaseTooFarAhead);

        let result = accept_migration_error(&connection, error);

        assert_eq!(result.is_ok(), has_newer_version);
    }
}
