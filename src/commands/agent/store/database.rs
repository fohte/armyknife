use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::commands::agent::error::CcError;
use crate::commands::agent::types::Session;

mod activity;
mod agent_files;
mod decisions;
mod deferred_stops;
mod initial_prompt;
mod initialization;
mod migration;
mod migrations;
mod one_time_permissions;
mod peer_out_messages;
mod permission_denied;
mod permission_requests;
mod proposals;
mod queued_prompts;
mod replacements;
mod resolution_lock;
mod session_closures;
mod todos;
mod unreported_stop_checks;
mod vibe_setting_revision;
mod wake;
mod web_state;

pub(crate) use activity::agent_response_timestamps;
pub(crate) use agent_files::AgentFile;
pub(crate) use agent_files::{agent_file, agent_files};
pub(crate) use decisions::{
    AgentDecision, AgentDecisionCounts, agent_decision_counts_by_session, agent_decisions,
    insert_agent_decision, set_agent_decision_review,
};
pub(crate) use deferred_stops::{
    DeferredStop, clear_deferred_stop, record_deferred_stop, take_mature_deferred_stops,
};
pub(crate) use initial_prompt::{clear_pending_initial_prompt, mark_pending_initial_prompt};
pub(crate) use one_time_permissions::{
    consume_permission_retry_approval, insert_permission_retry_approval,
    revoke_permission_retry_approval,
};
pub(crate) use peer_out_messages::WatchWebPeerOutMessage;
pub(crate) use peer_out_messages::{
    insert_watch_web_peer_out_message, mark_watch_web_peer_out_message_delivered,
    watch_web_peer_out_messages,
};
pub(crate) use permission_denied::{PermissionDeniedOperation, PermissionDeniedResolution};
pub(crate) use permission_denied::{
    add_permission_denied, permission_denied_operations,
    resolve_matching_unclassified_permission_denials, resolve_permission_denied,
};
pub(crate) use permission_requests::{
    PermissionRequestData, PermissionRequestDecision, PermissionRequestResolution,
    PermissionRequestWait, PermissionRequestWaiter, permission_request_data,
    resolve_permission_request, resolve_permission_request_with_answers,
};
pub(crate) use proposals::{AgentProposal, ProposalResolution};
pub(crate) use proposals::{
    add_agent_proposal, agent_proposals, edit_agent_proposal, lock_agent_proposal_resolution,
    resolve_agent_proposal, unanswered_agent_proposal_bodies, write_agent_proposal_answer_at,
};
pub(crate) use queued_prompts::{
    WatchWebActionEventKind, WatchWebQueuedAttachment, WatchWebQueuedPrompt,
};
pub(crate) use queued_prompts::{
    delete_watch_web_queued_prompts, insert_watch_web_queued_prompt,
    mark_watch_web_queued_prompts_resolved, watch_web_queued_prompt_message_id_exists,
    watch_web_queued_prompts,
};
pub(crate) use replacements::{
    all_unreplaced_sessions, resolve_current_session_id, save_rewind_sessions,
};
pub(crate) use session_closures::{clear_session_closed, closed_session_ids, mark_session_closed};
pub(crate) use todos::{AgentTodo, TodoResolution};
pub(crate) use todos::{
    add_agent_todo, agent_todos, edit_agent_todo, has_incomplete_todos,
    incomplete_agent_todo_titles, lock_agent_todo_resolution, resolve_agent_todo,
    resolve_agent_todo_at, set_agent_todo_folded,
};
pub(crate) use unreported_stop_checks::check_unreported_stop;
pub(crate) use web_state::WatchWebSessionState;
pub(crate) use web_state::set_watch_web_compacting;
pub(super) use web_state::{
    clear_watch_web_later, set_watch_web_later, set_watch_web_primary_task, set_watch_web_today,
    watch_web_session_start_generation, watch_web_session_states, watch_web_user_marker,
};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn lock_crit_port(sessions_dir: &Path, port: u16) -> Result<fs::File> {
    resolution_lock::lock_file(sessions_dir, "crit-port-locks", &format!("{port}.lock"))
}

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
    pending_compacting: Cell<Option<bool>>,
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
    open_connection_with_options(sessions_dir, BUSY_TIMEOUT, true)
}

pub(crate) fn open_usage_connection(sessions_dir: &Path) -> Result<Connection> {
    open_connection_with_options(sessions_dir, Duration::from_millis(100), false)
}

fn open_connection_with_options(
    sessions_dir: &Path,
    busy_timeout: Duration,
    import_legacy_data: bool,
) -> Result<Connection> {
    fs::create_dir_all(sessions_dir)
        .with_context(|| format!("creating sessions dir {}", sessions_dir.display()))?;

    let database_path = sessions_dir.join("sessions.db");
    let mut connection = Connection::open(&database_path)
        .with_context(|| format!("opening session database {}", database_path.display()))?;
    connection.busy_timeout(busy_timeout)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    initialization::run(&mut connection, sessions_dir, import_legacy_data)?;

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
        pending_compacting: Cell::new(None),
    })
}

pub(crate) fn claim_auto_title_generation(sessions_dir: &Path, session_id: &str) -> Result<bool> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let inserted = transaction.execute(
        "INSERT OR IGNORE INTO agent_title_generation_attempt (session_id) VALUES (?1)",
        [session_id],
    )?;
    transaction.commit()?;
    Ok(inserted == 1)
}

pub(crate) fn has_auto_title_generation_attempt(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<bool> {
    validate_session_id(session_id)?;
    let connection = open_connection(sessions_dir)?;
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_title_generation_attempt WHERE session_id = ?1)",
        [session_id],
        |row| row.get(0),
    )?)
}

pub(crate) fn release_auto_title_generation_claim(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<()> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
        "DELETE FROM agent_title_generation_attempt WHERE session_id = ?1",
        [session_id],
    )?;
    transaction.commit()?;
    Ok(())
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

    pub(crate) fn load_skipping_undecodable(&self) -> Result<Option<Session>> {
        read_session_skipping_undecodable(&self.connection, &self.session_id)
    }

    pub(crate) fn has_pending_agent_requests(&self) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM agent_proposal
                 WHERE session_id = ?1 AND resolved_at IS NULL
             ) OR EXISTS(
                 SELECT 1 FROM agent_todo
                 WHERE session_id = ?1 AND completed_at IS NULL
             )",
            [&self.session_id],
            |row| row.get(0),
        )?)
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

    pub(crate) fn consume_pending_initial_prompt(&self) -> Result<bool> {
        self.ensure_transaction()?;
        initial_prompt::consume_pending_initial_prompt(&self.connection, &self.session_id)
    }

    pub(crate) fn register_permission_request(
        &self,
        agent_key: &str,
        request_id: &str,
        tool_name: Option<&str>,
        tool_input: &serde_json::Value,
    ) -> Result<()> {
        self.ensure_transaction()?;
        permission_requests::insert_permission_request(
            &self.connection,
            &self.session_id,
            agent_key,
            request_id,
            tool_name,
            tool_input,
        )
    }

    pub(crate) fn remove_permission_request(&self, request_id: &str) -> Result<()> {
        self.ensure_transaction()?;
        self.connection.execute(
            "DELETE FROM permission_request_decision WHERE session_id = ?1 AND request_id = ?2",
            params![self.session_id, request_id],
        )?;
        Ok(())
    }

    pub(crate) fn queue_watch_web_compacting(&self, is_compacting: bool) {
        self.pending_compacting.set(Some(is_compacting));
    }

    pub(crate) fn record_watch_web_session_start(&self) -> Result<()> {
        self.ensure_transaction()?;
        web_state::record_watch_web_session_start_in_transaction(&self.connection, &self.session_id)
    }

    pub(crate) fn save(&self, session: &Session) -> Result<()> {
        ensure!(
            session.session_id == self.session_id,
            "session ID cannot change while its store lock is held"
        );
        self.ensure_transaction()?;
        write_session(&self.connection, session)?;
        self.apply_pending_watch_web_compacting()?;
        self.connection.execute_batch("COMMIT")?;
        self.transaction_active.set(false);
        Ok(())
    }

    pub(crate) fn record_agent_response_at(&self, responded_at: DateTime<Utc>) -> Result<()> {
        self.ensure_transaction()?;
        activity::record_agent_response_at(&self.connection, &self.session_id, responded_at)
    }

    pub(crate) fn save_vibe_setting(&self, session: &Session) -> Result<String> {
        ensure!(
            session.session_id == self.session_id,
            "session ID cannot change while its store lock is held"
        );
        self.ensure_transaction()?;
        let current_vibe_enabled = self.load()?.map(|current| current.vibe_enabled);
        let vibe_setting_changed = current_vibe_enabled
            .map(|current| current != session.vibe_enabled)
            .unwrap_or(session.vibe_enabled);
        write_session(&self.connection, session)?;
        self.apply_pending_watch_web_compacting()?;
        if !vibe_setting_changed {
            vibe_setting_revision::rotate(&self.connection, &self.session_id)?;
        }
        let revision = vibe_setting_revision::current(&self.connection, &self.session_id)?
            .ok_or_else(|| anyhow!("vibe setting token is missing after save"))?;
        self.connection.execute_batch("COMMIT")?;
        self.transaction_active.set(false);
        Ok(revision)
    }

    pub(crate) fn vibe_setting_revision(&self) -> Result<Option<String>> {
        vibe_setting_revision::current(&self.connection, &self.session_id)
    }

    pub(crate) fn delete(&self) -> Result<bool> {
        self.ensure_transaction()?;
        let removed = delete_session_row(&self.connection, &self.session_id)?;
        self.connection.execute_batch("COMMIT")?;
        self.transaction_active.set(false);
        Ok(removed)
    }

    pub(crate) fn has_incomplete_todos(&self) -> Result<bool> {
        has_incomplete_todos(&self.connection, &self.session_id)
    }

    fn ensure_transaction(&self) -> Result<()> {
        if !self.transaction_active.get() {
            self.connection.execute_batch("BEGIN IMMEDIATE")?;
            self.transaction_active.set(true);
        }
        Ok(())
    }

    fn apply_pending_watch_web_compacting(&self) -> Result<()> {
        if let Some(is_compacting) = self.pending_compacting.take() {
            web_state::set_watch_web_compacting_in_transaction(
                &self.connection,
                &self.session_id,
                is_compacting,
            )?;
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

/// Deletes a session unless an incomplete todo still needs the user's attention.
/// Returns false when a todo keeps the row available or the row was already absent.
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

pub(crate) fn session_revisions_by_session(sessions_dir: &Path) -> Result<HashMap<String, i64>> {
    if !sessions_dir.exists() {
        return Ok(HashMap::new());
    }
    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare("SELECT session_id, rev FROM sessions")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<rusqlite::Result<HashMap<_, _>>>()
        .map_err(Into::into)
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
    let stored_session = read_stored_session(connection, &session.session_id)?;
    ensure!(
        !matches!(stored_session, StoredSession::Undecodable),
        "refusing to overwrite undecodable session {}",
        session.session_id
    );
    let vibe_setting_changed = match &stored_session {
        StoredSession::Missing => session.vibe_enabled,
        StoredSession::Decoded(previous) => previous.vibe_enabled != session.vibe_enabled,
        StoredSession::Undecodable => false,
    };
    if vibe_setting_changed {
        vibe_setting_revision::rotate(connection, &session.session_id)?;
    }
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

fn bump_session_revision(connection: &Connection, session_id: &str) -> Result<i64> {
    let revision = increment_revision(connection)?;
    let updated = connection.execute(
        "UPDATE sessions SET rev = ?1 WHERE session_id = ?2",
        params![revision, session_id],
    )?;
    ensure!(
        updated == 1,
        "session {session_id} disappeared during update"
    );
    Ok(revision)
}

fn read_revision(connection: &Connection) -> Result<i64> {
    Ok(connection.query_row(
        "SELECT CAST(value AS INTEGER) FROM session_store_meta WHERE key = 'revision'",
        [],
        |row| row.get(0),
    )?)
}

/// Keeps sessions with incomplete todos available to watch-web.
/// Returns false when a todo keeps the row available or the row was already absent.
fn delete_session_row(connection: &Connection, session_id: &str) -> Result<bool> {
    if has_incomplete_todos(connection, session_id)? {
        return Ok(false);
    }

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
    use rstest::{fixture, rstest};
    use serde_json::Value;

    #[fixture]
    fn sessions_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write_legacy_session(sessions_dir: &Path, session_id: &str) {
        fs::write(
            sessions_dir.join(format!("{session_id}.json")),
            serde_json::json!({
                "session_id": session_id,
                "cwd": "/fixture/worktree",
                "transcript_path": null,
                "tmux_info": null,
                "status": "stopped",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "last_message": null,
            })
            .to_string(),
        )
        .unwrap();
    }

    fn stored_session_exists(connection: &Connection, session_id: &str) -> bool {
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
                [session_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[rstest]
    fn runs_migrations_once_for_each_sessions_directory(sessions_dir: tempfile::TempDir) {
        let sessions_dir = sessions_dir.path();
        drop(open_connection(sessions_dir).unwrap());

        let connection = Connection::open(sessions_dir.join("sessions.db")).unwrap();
        connection
            .pragma_update(None, "user_version", migrations::current_version() - 1)
            .unwrap();
        drop(connection);

        let connection = open_usage_connection(sessions_dir).unwrap();
        let user_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();

        assert_eq!(user_version, migrations::current_version() - 1);
    }

    #[rstest]
    fn retries_migrations_after_a_failed_attempt(sessions_dir: tempfile::TempDir) {
        let sessions_dir = sessions_dir.path();
        let connection = Connection::open(sessions_dir.join("sessions.db")).unwrap();
        connection
            .execute_batch("CREATE TABLE sessions (session_id TEXT PRIMARY KEY);")
            .unwrap();
        drop(connection);

        let first_attempt_failed = open_connection(sessions_dir).is_err();
        let connection = Connection::open(sessions_dir.join("sessions.db")).unwrap();
        connection.execute("DROP TABLE sessions", []).unwrap();
        drop(connection);

        let connection = open_connection(sessions_dir).unwrap();
        let user_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();

        assert_eq!(
            (first_attempt_failed, user_version),
            (true, migrations::current_version()),
        );
    }

    #[rstest]
    fn imports_legacy_json_once_for_each_sessions_directory(sessions_dir: tempfile::TempDir) {
        let sessions_dir = sessions_dir.path();
        write_legacy_session(sessions_dir, "fixture-first-session");

        let connection = open_connection(sessions_dir).unwrap();
        let first_session_exists = stored_session_exists(&connection, "fixture-first-session");
        drop(connection);

        write_legacy_session(sessions_dir, "fixture-later-session");
        let connection = open_connection(sessions_dir).unwrap();
        let later_session_exists = stored_session_exists(&connection, "fixture-later-session");

        assert_eq!((first_session_exists, later_session_exists), (true, false));
    }

    #[rstest]
    fn retries_legacy_import_after_a_failed_attempt(sessions_dir: tempfile::TempDir) {
        let sessions_dir = sessions_dir.path();
        let mut connection = Connection::open(sessions_dir.join("sessions.db")).unwrap();
        migrations::migrate_if_needed(&mut connection).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_fixture_legacy_import
                 BEFORE INSERT ON sessions
                 WHEN NEW.session_id = 'fixture-legacy-session'
                 BEGIN
                     SELECT RAISE(FAIL, 'fixture import failure');
                 END;",
            )
            .unwrap();
        drop(connection);
        write_legacy_session(sessions_dir, "fixture-legacy-session");

        let connection = open_connection(sessions_dir).unwrap();
        let exists_after_failed_attempt =
            stored_session_exists(&connection, "fixture-legacy-session");
        drop(connection);

        let connection = Connection::open(sessions_dir.join("sessions.db")).unwrap();
        connection
            .execute_batch("DROP TRIGGER fail_fixture_legacy_import;")
            .unwrap();
        drop(connection);

        let connection = open_connection(sessions_dir).unwrap();
        let exists_after_retry = stored_session_exists(&connection, "fixture-legacy-session");

        assert_eq!(
            (exists_after_failed_attempt, exists_after_retry),
            (false, true)
        );
    }

    #[rstest]
    fn retries_expired_legacy_cleanup_after_a_failed_attempt(sessions_dir: tempfile::TempDir) {
        let sessions_dir = sessions_dir.path();
        let mut connection = Connection::open(sessions_dir.join("sessions.db")).unwrap();
        migrations::migrate_if_needed(&mut connection).unwrap();
        connection
            .execute(
                "INSERT INTO session_store_meta (key, value) VALUES ('legacy-imported-at', ?1)",
                [(Utc::now() - chrono::TimeDelta::days(31)).to_rfc3339()],
            )
            .unwrap();
        drop(connection);
        let legacy_dir = sessions_dir.join("legacy-json");
        fs::write(&legacy_dir, "fixture file prevents directory cleanup").unwrap();

        drop(open_connection(sessions_dir).unwrap());
        let legacy_file_remains = legacy_dir.is_file();

        fs::remove_file(&legacy_dir).unwrap();
        fs::create_dir(&legacy_dir).unwrap();
        fs::write(legacy_dir.join("fixture.json"), "fixture backup").unwrap();
        drop(open_connection(sessions_dir).unwrap());

        assert_eq!((legacy_file_remains, legacy_dir.exists()), (true, false));
    }

    #[test]
    fn initializes_each_sessions_directory_once() {
        let first_sessions_dir = tempfile::tempdir().unwrap();
        let second_sessions_dir = tempfile::tempdir().unwrap();
        write_legacy_session(first_sessions_dir.path(), "fixture-first-directory");
        write_legacy_session(second_sessions_dir.path(), "fixture-second-directory");

        let first_connection = open_connection(first_sessions_dir.path()).unwrap();
        let second_connection = open_connection(second_sessions_dir.path()).unwrap();
        let actual = (
            stored_session_exists(&first_connection, "fixture-first-directory"),
            stored_session_exists(&second_connection, "fixture-second-directory"),
        );

        assert_eq!(actual, (true, true));
    }

    #[rstest]
    fn imports_legacy_json_after_a_usage_only_first_open(sessions_dir: tempfile::TempDir) {
        let sessions_dir = sessions_dir.path();
        write_legacy_session(sessions_dir, "fixture-usage-first-session");

        drop(open_usage_connection(sessions_dir).unwrap());
        let connection = open_connection(sessions_dir).unwrap();
        let legacy_backup_exists = sessions_dir
            .join("legacy-json")
            .join("fixture-usage-first-session.json")
            .exists();

        assert_eq!(
            (
                stored_session_exists(&connection, "fixture-usage-first-session"),
                legacy_backup_exists,
            ),
            (true, true),
        );
    }

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

    #[test]
    fn save_rewind_sessions_copies_task_and_user_marker_state() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut previous: Session = serde_json::from_value(serde_json::json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/worktree",
            "transcript_path": null,
            "tmux_info": null,
            "status": "paused",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .unwrap();
        previous.status = crate::commands::agent::types::SessionStatus::Ended;
        previous.ancestor_session_ids = vec!["fixture-parent".to_string()];
        let mut forked = previous.clone();
        forked.session_id = "fixture-fork".to_string();
        forked.status = crate::commands::agent::types::SessionStatus::Stopped;
        let mut child = previous.clone();
        child.session_id = "fixture-child".to_string();
        child.ancestor_session_ids =
            vec!["fixture-parent".to_string(), previous.session_id.clone()];
        let mut grandchild = previous.clone();
        grandchild.session_id = "fixture-grandchild".to_string();
        grandchild.ancestor_session_ids = vec![
            "fixture-parent".to_string(),
            previous.session_id.clone(),
            child.session_id.clone(),
        ];

        let connection = open_connection(temp_dir.path()).unwrap();
        connection
            .execute(
                "INSERT INTO watch_web_session_state
                    (session_id, is_today, primary_task_id, watch_web_user_marker)
                 VALUES (?1, 1, ?2, ?3)",
                params![previous.session_id, "fixture-task", "fixture-marker"],
            )
            .unwrap();
        drop(connection);
        save_session(temp_dir.path(), &child).unwrap();
        save_session(temp_dir.path(), &grandchild).unwrap();

        save_rewind_sessions(temp_dir.path(), &previous, &forked).unwrap();

        let previous_after = load_session(temp_dir.path(), &previous.session_id)
            .unwrap()
            .unwrap();
        let replacement =
            resolve_current_session_id(temp_dir.path(), &previous.session_id).unwrap();
        let forked_after = load_session(temp_dir.path(), &forked.session_id)
            .unwrap()
            .unwrap();
        let child_after = load_session(temp_dir.path(), &child.session_id)
            .unwrap()
            .unwrap();
        let grandchild_after = load_session(temp_dir.path(), &grandchild.session_id)
            .unwrap()
            .unwrap();
        let connection = open_connection(temp_dir.path()).unwrap();
        let copied_state = connection
            .query_row(
                "SELECT is_today, primary_task_id, watch_web_user_marker
                 FROM watch_web_session_state WHERE session_id = ?1",
                [&forked.session_id],
                |row| {
                    Ok((
                        row.get::<_, bool>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(
            (
                previous_after.status,
                replacement,
                forked_after.session_id,
                forked_after.ancestor_session_ids,
                child_after.ancestor_session_ids,
                grandchild_after.ancestor_session_ids,
                copied_state,
            ),
            (
                crate::commands::agent::types::SessionStatus::Ended,
                "fixture-fork".to_string(),
                "fixture-fork".to_string(),
                vec!["fixture-parent".to_string()],
                vec!["fixture-parent".to_string(), "fixture-fork".to_string()],
                vec![
                    "fixture-parent".to_string(),
                    "fixture-fork".to_string(),
                    "fixture-child".to_string(),
                ],
                (
                    true,
                    Some("fixture-task".to_string()),
                    Some("fixture-marker".to_string())
                ),
            ),
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
