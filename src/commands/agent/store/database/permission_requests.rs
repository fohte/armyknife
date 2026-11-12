use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{open_connection, read_session_strict, write_session};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "behavior", rename_all = "snake_case")]
pub(crate) enum PermissionRequestDecision {
    Allow,
    Deny { message: String, interrupt: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PermissionRequestData {
    pub(crate) tool_name: Option<String>,
    pub(crate) tool_input: Value,
    pub(crate) pending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PermissionRequestWait {
    Pending,
    Finished,
    Decision(PermissionRequestDecision),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionRequestResolution {
    Resolved,
    Missing,
    Finished,
}

pub(crate) struct PermissionRequestWaiter {
    connection: Connection,
}

impl PermissionRequestWaiter {
    pub(crate) fn open(sessions_dir: &Path) -> Result<Self> {
        Ok(Self {
            connection: open_connection(sessions_dir)?,
        })
    }

    pub(crate) fn poll(
        &mut self,
        session_id: &str,
        agent_key: &str,
        request_id: &str,
    ) -> Result<PermissionRequestWait> {
        poll_permission_request_from(&mut self.connection, session_id, agent_key, request_id)
    }

    pub(crate) fn expire_with_deny(
        &mut self,
        session_id: &str,
        agent_key: &str,
        request_id: &str,
        message: &str,
    ) -> Result<PermissionRequestWait> {
        finalize_permission_request_from(
            &mut self.connection,
            session_id,
            agent_key,
            request_id,
            true,
            Some(&PermissionRequestDecision::Deny {
                message: message.to_string(),
                interrupt: false,
            }),
        )
    }
}

pub(super) fn insert_permission_request(
    connection: &Connection,
    session_id: &str,
    agent_key: &str,
    request_id: &str,
    tool_name: Option<&str>,
    tool_input: &Value,
) -> Result<()> {
    connection.execute(
        "DELETE FROM permission_request_decision WHERE session_id = ?1 AND agent_key = ?2",
        params![session_id, agent_key],
    )?;
    connection.execute(
        "INSERT INTO permission_request_decision
            (request_id, session_id, agent_key, tool_name, tool_input)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            request_id,
            session_id,
            agent_key,
            tool_name,
            serde_json::to_string(tool_input)?,
        ],
    )?;
    Ok(())
}

pub(crate) fn permission_request_data(
    sessions_dir: &Path,
    session_id: &str,
    request_id: &str,
) -> Result<Option<PermissionRequestData>> {
    super::validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(None);
    }
    let connection = open_connection(sessions_dir)?;
    connection
        .query_row(
            "SELECT tool_name, tool_input, decision IS NULL
             FROM permission_request_decision
             WHERE session_id = ?1 AND request_id = ?2",
            params![session_id, request_id],
            |row| {
                let tool_name = row.get(0)?;
                let tool_input = row.get::<_, String>(1)?;
                let pending = row.get(2)?;
                Ok((tool_name, tool_input, pending))
            },
        )
        .optional()?
        .map(|(tool_name, tool_input, pending)| {
            Ok(PermissionRequestData {
                tool_name,
                tool_input: serde_json::from_str(&tool_input)?,
                pending,
            })
        })
        .transpose()
}

pub(crate) fn resolve_permission_request(
    sessions_dir: &Path,
    session_id: &str,
    agent_key: &str,
    request_id: &str,
    decision: &PermissionRequestDecision,
) -> Result<PermissionRequestResolution> {
    super::validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(mut session) = read_session_strict(&transaction, session_id)? else {
        transaction.commit()?;
        return Ok(PermissionRequestResolution::Finished);
    };
    if !permission_request_is_current(&session, agent_key, request_id) {
        transaction.commit()?;
        return Ok(PermissionRequestResolution::Finished);
    }

    let current_decision = transaction
        .query_row(
            "SELECT decision FROM permission_request_decision
             WHERE session_id = ?1 AND agent_key = ?2 AND request_id = ?3",
            params![session_id, agent_key, request_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    let Some(current_decision) = current_decision else {
        transaction.commit()?;
        return Ok(PermissionRequestResolution::Missing);
    };
    if current_decision.is_some() {
        transaction.commit()?;
        return Ok(PermissionRequestResolution::Finished);
    }

    let changed = transaction.execute(
        "UPDATE permission_request_decision
         SET decision = ?1
         WHERE session_id = ?2 AND request_id = ?3 AND decision IS NULL",
        params![serde_json::to_string(decision)?, session_id, request_id,],
    )?;
    if changed != 1 {
        transaction.commit()?;
        return Ok(PermissionRequestResolution::Finished);
    }

    session.updated_at = Utc::now();
    write_session(&transaction, &session)?;
    transaction.commit()?;
    Ok(PermissionRequestResolution::Resolved)
}

fn poll_permission_request_from(
    connection: &mut Connection,
    session_id: &str,
    agent_key: &str,
    request_id: &str,
) -> Result<PermissionRequestWait> {
    super::validate_session_id(session_id)?;
    let decision = connection
        .query_row(
            "SELECT decision FROM permission_request_decision
             WHERE session_id = ?1 AND agent_key = ?2 AND request_id = ?3",
            params![session_id, agent_key, request_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    let Some(decision) = decision else {
        return Ok(PermissionRequestWait::Finished);
    };
    let session = read_session_strict(connection, session_id)?;
    let is_current = session
        .as_ref()
        .is_some_and(|session| permission_request_is_current(session, agent_key, request_id));
    if is_current && decision.is_none() {
        return Ok(PermissionRequestWait::Pending);
    }

    finalize_permission_request_from(connection, session_id, agent_key, request_id, false, None)
}

fn finalize_permission_request_from(
    connection: &mut Connection,
    session_id: &str,
    agent_key: &str,
    request_id: &str,
    expire_pending: bool,
    expiration_decision: Option<&PermissionRequestDecision>,
) -> Result<PermissionRequestWait> {
    super::validate_session_id(session_id)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let session = read_session_strict(&transaction, session_id)?;
    let is_current = session
        .as_ref()
        .is_some_and(|session| permission_request_is_current(session, agent_key, request_id));
    let decision = transaction
        .query_row(
            "SELECT decision FROM permission_request_decision
             WHERE session_id = ?1 AND agent_key = ?2 AND request_id = ?3",
            params![session_id, agent_key, request_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    let Some(decision) = decision else {
        transaction.commit()?;
        return Ok(PermissionRequestWait::Finished);
    };
    if !is_current {
        transaction.execute(
            "DELETE FROM permission_request_decision
             WHERE session_id = ?1 AND agent_key = ?2 AND request_id = ?3",
            params![session_id, agent_key, request_id],
        )?;
        transaction.commit()?;
        return Ok(PermissionRequestWait::Finished);
    }
    if decision.is_none() && expire_pending {
        transaction.execute(
            "DELETE FROM permission_request_decision
             WHERE session_id = ?1 AND agent_key = ?2 AND request_id = ?3",
            params![session_id, agent_key, request_id],
        )?;
        transaction.commit()?;
        return Ok(
            expiration_decision.map_or(PermissionRequestWait::Finished, |decision| {
                PermissionRequestWait::Decision(decision.clone())
            }),
        );
    }
    let Some(decision) = decision else {
        transaction.commit()?;
        return Ok(PermissionRequestWait::Pending);
    };
    let decision = serde_json::from_str(&decision)?;
    transaction.execute(
        "DELETE FROM permission_request_decision
         WHERE session_id = ?1 AND agent_key = ?2 AND request_id = ?3",
        params![session_id, agent_key, request_id],
    )?;
    transaction.commit()?;
    Ok(PermissionRequestWait::Decision(decision))
}

fn permission_request_is_current(
    session: &crate::commands::agent::types::Session,
    agent_key: &str,
    request_id: &str,
) -> bool {
    session.pending_permission_agent_ids.contains(agent_key)
        && session
            .pending_permission_request_ids
            .get(agent_key)
            .map(String::as_str)
            == Some(request_id)
        && session.status == crate::commands::agent::types::SessionStatus::WaitingInput
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;
    use std::time::Duration;

    use chrono::{TimeZone, Utc};
    use rstest::fixture;
    use tempfile::TempDir;

    use super::*;
    use crate::commands::agent::types::{MAIN_THREAD_AGENT_KEY, Session, SessionStatus};

    fn sessions_dir() -> TempDir {
        tempfile::tempdir().expect("temporary sessions directory should be created")
    }

    fn session(session_id: &str) -> Session {
        Session {
            session_id: session_id.to_string(),
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/example"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::WaitingInput,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: Utc.timestamp_opt(1, 0).single().unwrap(),
            updated_at: Utc.timestamp_opt(1, 0).single().unwrap(),
            last_message: None,
            current_tool: Some("Bash".to_string()),
            label: None,
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::from([MAIN_THREAD_AGENT_KEY.to_string()]),
            pending_permission_request_ids: BTreeMap::from([(
                MAIN_THREAD_AGENT_KEY.to_string(),
                "request-cobalt".to_string(),
            )]),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: crate::commands::agent::types::Engine::Claude,
            model: None,
            effort: None,
        }
    }

    #[fixture]
    fn pending_bash_request() -> TempDir {
        let sessions_dir = sessions_dir();
        super::super::super::save_session_to(sessions_dir.path(), &session("session-cobalt"))
            .expect("session should be saved");
        let mut connection = open_connection(sessions_dir.path()).expect("database should open");
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("transaction should begin");
        insert_permission_request(
            &transaction,
            "session-cobalt",
            MAIN_THREAD_AGENT_KEY,
            "request-cobalt",
            Some("Bash"),
            &serde_json::json!({"command": "echo fictional"}),
        )
        .expect("request should be registered");
        transaction.commit().expect("request should be committed");
        sessions_dir
    }

    fn poll_request(
        sessions_dir: &Path,
        session_id: &str,
        request_id: &str,
    ) -> PermissionRequestWait {
        PermissionRequestWaiter::open(sessions_dir)
            .expect("waiter should open")
            .poll(session_id, MAIN_THREAD_AGENT_KEY, request_id)
            .expect("request should be polled")
    }

    #[test]
    fn permission_decision_is_available_to_the_matching_hook_request() {
        let sessions_dir = sessions_dir();
        super::super::super::save_session_to(sessions_dir.path(), &session("session-cobalt"))
            .expect("session should be saved");
        let mut connection = open_connection(sessions_dir.path()).expect("database should open");
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("transaction should begin");
        insert_permission_request(
            &transaction,
            "session-cobalt",
            MAIN_THREAD_AGENT_KEY,
            "request-cobalt",
            Some("Bash"),
            &serde_json::json!({"command": "echo fictional"}),
        )
        .expect("request should be registered");
        transaction.commit().expect("request should be committed");
        let decision = PermissionRequestDecision::Deny {
            message: "Rejected from watch-web.".to_string(),
            interrupt: false,
        };
        assert_eq!(
            resolve_permission_request(
                sessions_dir.path(),
                "session-cobalt",
                MAIN_THREAD_AGENT_KEY,
                "request-cobalt",
                &decision,
            )
            .expect("decision should be saved"),
            PermissionRequestResolution::Resolved,
        );

        assert_eq!(
            poll_request(sessions_dir.path(), "session-cobalt", "request-cobalt"),
            PermissionRequestWait::Decision(decision),
        );
    }

    #[rstest::rstest]
    fn terminal_resolution_ends_wait_without_a_hook_decision(pending_bash_request: TempDir) {
        let connection =
            open_connection(pending_bash_request.path()).expect("database should open");
        let mut session = read_session_strict(&connection, "session-cobalt")
            .expect("session should load")
            .expect("session should exist");
        session.pending_permission_agent_ids.clear();
        session.pending_permission_request_ids.clear();
        super::super::super::save_session_to(pending_bash_request.path(), &session)
            .expect("session should be saved");
        let mut waiter =
            PermissionRequestWaiter::open(pending_bash_request.path()).expect("waiter should open");

        assert_eq!(
            waiter
                .expire_with_deny(
                    "session-cobalt",
                    MAIN_THREAD_AGENT_KEY,
                    "request-cobalt",
                    "automatic denial fixture",
                )
                .expect("request expiration should finish"),
            PermissionRequestWait::Finished,
        );
    }

    #[rstest::rstest]
    fn pending_poll_does_not_acquire_a_write_lock(pending_bash_request: TempDir) {
        let mut waiter =
            PermissionRequestWaiter::open(pending_bash_request.path()).expect("waiter should open");
        waiter
            .connection
            .busy_timeout(Duration::ZERO)
            .expect("poll should not wait for a writer");
        let mut writer =
            open_connection(pending_bash_request.path()).expect("database should open");
        let transaction = writer
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("writer lock should be acquired");

        let wait = waiter
            .poll("session-cobalt", MAIN_THREAD_AGENT_KEY, "request-cobalt")
            .expect("read-only polling should work while a writer is active");
        transaction
            .commit()
            .expect("writer lock should be released");

        assert_eq!(wait, PermissionRequestWait::Pending);
    }

    #[rstest::rstest]
    fn expiring_a_pending_hook_request_returns_a_non_interrupting_deny(
        pending_bash_request: TempDir,
    ) {
        let mut waiter =
            PermissionRequestWaiter::open(pending_bash_request.path()).expect("waiter should open");
        let wait = waiter
            .expire_with_deny(
                "session-cobalt",
                MAIN_THREAD_AGENT_KEY,
                "request-cobalt",
                "No response was provided in watch-web or the terminal within 590 seconds. This request was automatically denied.",
            )
            .expect("request should expire with a deny");
        let request = permission_request_data(
            pending_bash_request.path(),
            "session-cobalt",
            "request-cobalt",
        )
        .expect("request should load after expiration");

        assert_eq!(
            (wait, request),
            (
                PermissionRequestWait::Decision(PermissionRequestDecision::Deny {
                    message: "No response was provided in watch-web or the terminal within 590 seconds. This request was automatically denied.".to_string(),
                    interrupt: false,
                }),
                None,
            ),
        );
    }

    #[rstest::rstest]
    #[case::stopped(SessionStatus::Stopped)]
    #[case::paused(SessionStatus::Paused)]
    fn a_session_that_is_no_longer_waiting_ends_the_hook(#[case] status: SessionStatus) {
        let sessions_dir = sessions_dir();
        let mut session = session("session-cobalt");
        session.status = status;
        super::super::super::save_session_to(sessions_dir.path(), &session)
            .expect("session should be saved");

        assert_eq!(
            poll_request(sessions_dir.path(), "session-cobalt", "request-cobalt"),
            PermissionRequestWait::Finished,
        );
    }
}
