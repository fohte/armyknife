use std::sync::LazyLock;

use anyhow::{Context, Result};
use rusqlite::Connection;
use rusqlite_migration::{Error as MigrationError, M, MigrationDefinitionError, Migrations};

// Keep older binaries compatible: new columns need defaults or must be nullable.
const AGENT_FILE_SCHEMA: &str = "CREATE TABLE agent_file (
    session_id TEXT NOT NULL,
    id TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    name TEXT NOT NULL,
    caption TEXT,
    mime_type TEXT,
    size INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (session_id, id)
);
CREATE INDEX agent_file_session_id_created_at
    ON agent_file (session_id, created_at, position);
CREATE TRIGGER agent_file_delete_session
    AFTER DELETE ON sessions
    BEGIN
        DELETE FROM agent_file WHERE session_id = OLD.session_id;
    END;";

static MIGRATION_ENTRIES: [M<'static>; 35] = [
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
        );",
    ),
    M::up(
        "CREATE TABLE IF NOT EXISTS watch_web_peer_out_message (
            id INTEGER PRIMARY KEY,
            from_session_id TEXT NOT NULL,
            to_session_id TEXT NOT NULL,
            content TEXT NOT NULL,
            sent_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS watch_web_peer_out_message_sender_target_time
            ON watch_web_peer_out_message (from_session_id, to_session_id, sent_at, id);",
    ),
    M::up("ALTER TABLE watch_web_session_state ADD COLUMN primary_task_id TEXT;"),
    M::up(
        "CREATE TABLE permission_request_decision (
            request_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            agent_key TEXT NOT NULL,
            tool_name TEXT,
            tool_input TEXT NOT NULL,
            decision TEXT
        );
        CREATE INDEX permission_request_decision_session_agent
            ON permission_request_decision (session_id, agent_key);
        CREATE TRIGGER permission_request_decision_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM permission_request_decision WHERE session_id = OLD.session_id;
            END;",
    ),
    M::up("ALTER TABLE permission_request_decision ADD COLUMN answers TEXT;"),
    M::up(
        "CREATE TABLE agent_todo (
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            title TEXT NOT NULL,
            detail TEXT,
            created_at TEXT NOT NULL,
            completed_at TEXT,
            comment TEXT,
            PRIMARY KEY (session_id, seq)
        );
        CREATE INDEX agent_todo_incomplete_session_id
            ON agent_todo (session_id) WHERE completed_at IS NULL;
        CREATE TRIGGER agent_todo_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM agent_todo WHERE session_id = OLD.session_id;
            END;",
    ),
    M::up(
        "CREATE TABLE agent_permission_denied (
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            tool_use_id TEXT,
            tool_name TEXT NOT NULL,
            tool_input TEXT NOT NULL,
            reason TEXT,
            created_at TEXT NOT NULL,
            resolved_at TEXT,
            PRIMARY KEY (session_id, seq),
            UNIQUE (session_id, tool_use_id)
        );
        CREATE INDEX agent_permission_denied_session_seq
            ON agent_permission_denied (session_id, seq);
        CREATE TRIGGER agent_permission_denied_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM agent_permission_denied WHERE session_id = OLD.session_id;
            END;",
    ),
    M::up(
        "ALTER TABLE watch_web_peer_out_message
         ADD COLUMN failed INTEGER NOT NULL DEFAULT 0;",
    ),
    M::up("ALTER TABLE agent_todo ADD COLUMN resolution TEXT;"),
    M::up(
        "ALTER TABLE watch_web_queued_prompt
         ADD COLUMN attachments TEXT NOT NULL DEFAULT '[]';
         ALTER TABLE watch_web_queued_prompt
         ADD COLUMN message_id TEXT;",
    ),
    M::up(
        "ALTER TABLE watch_web_queued_prompt
         ADD COLUMN resolved INTEGER NOT NULL DEFAULT 1;
         UPDATE watch_web_queued_prompt
         SET resolved = CASE WHEN message_id IS NULL THEN 1 ELSE 0 END;",
    ),
    M::up(AGENT_FILE_SCHEMA),
    M::up(
        "CREATE TABLE session_replacement (
            session_id TEXT PRIMARY KEY,
            replaced_by_session_id TEXT NOT NULL
        );",
    ),
    M::up(
        "ALTER TABLE agent_proposal ADD COLUMN resolution TEXT;
         ALTER TABLE agent_proposal ADD COLUMN comment TEXT;",
    ),
    M::up(
        "CREATE TABLE pending_initial_prompt (
            session_id TEXT PRIMARY KEY,
            created_at INTEGER NOT NULL
        );",
    ),
    M::up(
        "CREATE TABLE agent_session_usage (
            session_id TEXT PRIMARY KEY,
            engine TEXT NOT NULL,
            data TEXT NOT NULL,
            observed_at INTEGER NOT NULL
        );
        CREATE TABLE agent_engine_quota (
            engine TEXT PRIMARY KEY,
            data TEXT NOT NULL,
            observed_at INTEGER NOT NULL
        );",
    ),
    M::up(
        "CREATE TABLE session_vibe_setting_revision (
            session_id TEXT PRIMARY KEY,
            revision TEXT NOT NULL
        );
        CREATE TRIGGER session_vibe_setting_revision_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM session_vibe_setting_revision WHERE session_id = OLD.session_id;
            END;",
    ),
    M::up(
        "CREATE TABLE agent_title_generation_attempt (
            session_id TEXT PRIMARY KEY
        );
        CREATE TRIGGER agent_title_generation_attempt_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM agent_title_generation_attempt WHERE session_id = OLD.session_id;
            END;",
    ),
    M::up("ALTER TABLE agent_permission_denied ADD COLUMN resolution TEXT;"),
    M::up(
        "CREATE TABLE agent_permission_retry_approval (
            id INTEGER PRIMARY KEY,
            session_id TEXT NOT NULL,
            tool_name TEXT NOT NULL,
            tool_input TEXT NOT NULL,
            expires_at INTEGER NOT NULL
        );
        CREATE INDEX agent_permission_retry_approval_match
            ON agent_permission_retry_approval (session_id, tool_name, expires_at);
        CREATE TRIGGER agent_permission_retry_approval_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM agent_permission_retry_approval WHERE session_id = OLD.session_id;
            END;",
    ),
    M::up(
        "CREATE INDEX watch_web_queued_prompt_session_message_id
            ON watch_web_queued_prompt (session_id, message_id)
            WHERE message_id IS NOT NULL;",
    ),
    M::up(
        "CREATE TABLE agent_unreported_stop_peer_cursor (
            session_id TEXT PRIMARY KEY,
            last_message_id INTEGER NOT NULL
        );
        CREATE TRIGGER agent_unreported_stop_peer_cursor_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM agent_unreported_stop_peer_cursor WHERE session_id = OLD.session_id;
            END;",
    ),
    M::up(
        "ALTER TABLE agent_todo ADD COLUMN folded INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE agent_todo ADD COLUMN added_folded INTEGER NOT NULL DEFAULT 0;",
    ),
    M::up(
        "CREATE TABLE agent_session_close (
            session_id TEXT PRIMARY KEY
        );
        CREATE TRIGGER agent_session_close_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM agent_session_close WHERE session_id = OLD.session_id;
            END;",
    ),
    M::up("ALTER TABLE watch_web_queued_prompt ADD COLUMN action_event_kind TEXT;"),
    M::up(
        "CREATE TABLE agent_session_activity (
            session_id TEXT PRIMARY KEY,
            last_responded_at INTEGER NOT NULL
        );
        CREATE TRIGGER agent_session_activity_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM agent_session_activity WHERE session_id = OLD.session_id;
            END;",
    ),
    M::up(
        "CREATE TABLE agent_deferred_unreported_stop (
            session_id TEXT PRIMARY KEY,
            parent_session_id TEXT NOT NULL,
            stopped_at TEXT NOT NULL
        );
        CREATE TRIGGER agent_deferred_unreported_stop_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM agent_deferred_unreported_stop WHERE session_id = OLD.session_id;
            END;",
    ),
    M::up(
        "CREATE TABLE watch_web_session_start (
            session_id TEXT PRIMARY KEY,
            generation INTEGER NOT NULL
        );
        CREATE TRIGGER watch_web_session_start_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM watch_web_session_start WHERE session_id = OLD.session_id;
            END;",
    ),
    M::up(
        "CREATE TABLE agent_session_pane_binding_state (
            session_id TEXT PRIMARY KEY,
            has_parent_agent INTEGER NOT NULL CHECK (has_parent_agent IN (0, 1)),
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE TRIGGER agent_session_pane_binding_state_delete_session
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM agent_session_pane_binding_state WHERE session_id = OLD.session_id;
            END;",
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

#[cfg(test)]
mod tests;
