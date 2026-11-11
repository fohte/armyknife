use std::sync::LazyLock;

use anyhow::{Context, Result};
use rusqlite::Connection;
use rusqlite_migration::{Error as MigrationError, M, MigrationDefinitionError, Migrations};

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

pub(super) fn current_version() -> i64 {
    MIGRATION_ENTRIES.len() as i64
}

pub(super) fn migrate_if_needed(connection: &mut Connection) -> Result<()> {
    let user_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if user_version < current_version()
        && let Err(error) = MIGRATIONS.to_latest(connection)
    {
        accept_migration_error(connection, error)?;
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
        if user_version > current_version() {
            return Ok(());
        }
    }

    Err(error).context("migrating session database")
}
