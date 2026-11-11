use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{TransactionBehavior, params, params_from_iter};

use super::{increment_revision, open_connection, validate_session_id};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentProposal {
    pub(crate) session_id: String,
    pub(crate) seq: i64,
    pub(crate) body: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) answer: Option<String>,
    pub(crate) resolved_at: Option<DateTime<Utc>>,
}

pub(crate) fn add_agent_proposal(sessions_dir: &Path, session_id: &str, body: &str) -> Result<i64> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = Utc::now().to_rfc3339();
    transaction.execute(
        "UPDATE agent_proposal SET resolved_at = ?1
         WHERE session_id = ?2 AND resolved_at IS NULL",
        params![now, session_id],
    )?;
    let seq: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM agent_proposal WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO agent_proposal (session_id, seq, body, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![session_id, seq, body, now],
    )?;
    increment_revision(&transaction)?;
    transaction.commit()?;
    Ok(seq)
}

pub(crate) fn close_unanswered_agent_proposals(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<bool> {
    validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(false);
    }

    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE agent_proposal SET resolved_at = ?1
         WHERE session_id = ?2 AND resolved_at IS NULL",
        params![Utc::now().to_rfc3339(), session_id],
    )?;
    if changed > 0 {
        increment_revision(&transaction)?;
    }
    transaction.commit()?;
    Ok(changed > 0)
}

pub(crate) fn unanswered_agent_proposal_bodies(
    sessions_dir: &Path,
    session_ids: &[String],
) -> Result<Vec<(String, String)>> {
    if !sessions_dir.exists() || session_ids.is_empty() {
        return Ok(Vec::new());
    }

    let connection = open_connection(sessions_dir)?;
    let placeholders = (1..=session_ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!(
        "SELECT session_id, body FROM agent_proposal
         WHERE resolved_at IS NULL AND session_id IN ({placeholders})
         ORDER BY session_id, seq"
    );
    let mut statement = connection.prepare(&query)?;
    let rows = statement.query_map(params_from_iter(session_ids.iter()), |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(crate) fn agent_proposals(sessions_dir: &Path, session_id: &str) -> Result<Vec<AgentProposal>> {
    validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare(
        "SELECT session_id, seq, body, created_at, answer, resolved_at
         FROM agent_proposal WHERE session_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map([session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })?;
    rows.map(|row| {
        let (session_id, seq, body, created_at, answer, resolved_at) = row?;
        Ok(AgentProposal {
            session_id,
            seq,
            body,
            created_at: DateTime::parse_from_rfc3339(&created_at)
                .context("parsing proposal created_at")?
                .with_timezone(&Utc),
            answer,
            resolved_at: resolved_at
                .map(|value| {
                    DateTime::parse_from_rfc3339(&value)
                        .context("parsing proposal resolved_at")
                        .map(|parsed| parsed.with_timezone(&Utc))
                })
                .transpose()?,
        })
    })
    .collect()
}

pub(crate) fn write_agent_proposal_answer(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    answer: &str,
) -> Result<bool> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE agent_proposal SET answer = ?1, resolved_at = ?2
         WHERE session_id = ?3 AND seq = ?4 AND resolved_at IS NULL",
        params![answer, Utc::now().to_rfc3339(), session_id, seq],
    )?;
    if changed > 0 {
        increment_revision(&transaction)?;
    }
    transaction.commit()?;
    Ok(changed > 0)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::{
        AgentProposal, add_agent_proposal, agent_proposals, close_unanswered_agent_proposals,
        unanswered_agent_proposal_bodies, write_agent_proposal_answer,
    };
    use crate::commands::agent::store::session_store_revision_from;

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().expect("temporary sessions directory should be created")
    }

    fn normalize_timestamps(mut proposals: Vec<AgentProposal>) -> Vec<AgentProposal> {
        let epoch = Utc
            .timestamp_opt(0, 0)
            .single()
            .expect("epoch should be valid");
        proposals.iter_mut().for_each(|proposal| {
            proposal.created_at = epoch;
            proposal.resolved_at = proposal.resolved_at.map(|_| epoch);
        });
        proposals
    }

    #[rstest]
    fn adds_proposals_in_sequence_and_resolves_the_previous_unanswered_proposal(
        sessions_dir: TempDir,
    ) {
        let first = add_agent_proposal(sessions_dir.path(), "session-a", "first body")
            .expect("first proposal should save");
        let second = add_agent_proposal(sessions_dir.path(), "session-a", "second body")
            .expect("second proposal should save");
        let other_session = add_agent_proposal(sessions_dir.path(), "session-b", "other body")
            .expect("other session proposal should save");

        let proposals = normalize_timestamps(
            agent_proposals(sessions_dir.path(), "session-a").expect("proposals should load"),
        );
        let unanswered_bodies = unanswered_agent_proposal_bodies(
            sessions_dir.path(),
            &["session-a".to_string(), "session-b".to_string()],
        )
        .expect("unanswered proposal bodies should load");
        let epoch = Utc
            .timestamp_opt(0, 0)
            .single()
            .expect("epoch should be valid");

        assert_eq!(
            ((first, second, other_session), proposals, unanswered_bodies),
            (
                (1, 2, 1),
                vec![
                    AgentProposal {
                        session_id: "session-a".to_string(),
                        seq: 1,
                        body: "first body".to_string(),
                        created_at: epoch,
                        answer: None,
                        resolved_at: Some(epoch),
                    },
                    AgentProposal {
                        session_id: "session-a".to_string(),
                        seq: 2,
                        body: "second body".to_string(),
                        created_at: epoch,
                        answer: None,
                        resolved_at: None,
                    },
                ],
                vec![
                    ("session-a".to_string(), "second body".to_string()),
                    ("session-b".to_string(), "other body".to_string()),
                ],
            ),
        );
    }

    #[rstest]
    fn closing_unanswered_proposals_advances_revision_only_when_rows_change(sessions_dir: TempDir) {
        add_agent_proposal(sessions_dir.path(), "session-a", "body").expect("proposal should save");
        let before =
            session_store_revision_from(sessions_dir.path()).expect("revision should load");

        let changed = close_unanswered_agent_proposals(sessions_dir.path(), "session-a")
            .expect("unanswered proposal should close");
        let after_close =
            session_store_revision_from(sessions_dir.path()).expect("revision should load");
        let changed_again = close_unanswered_agent_proposals(sessions_dir.path(), "session-a")
            .expect("already resolved proposal should remain closed");
        let after_second_close =
            session_store_revision_from(sessions_dir.path()).expect("revision should load");

        assert_eq!(
            (changed, after_close, changed_again, after_second_close),
            (true, before + 1, false, after_close),
        );
    }

    #[rstest]
    fn answer_does_not_overwrite_a_resolved_proposal(sessions_dir: TempDir) {
        add_agent_proposal(sessions_dir.path(), "session-a", "body").expect("proposal should save");
        close_unanswered_agent_proposals(sessions_dir.path(), "session-a")
            .expect("proposal should close");
        let answer = r#"{"choice":0,"comment":"use this"}"#;

        let updated = write_agent_proposal_answer(sessions_dir.path(), "session-a", 1, answer)
            .expect("answer should write");
        let proposals = normalize_timestamps(
            agent_proposals(sessions_dir.path(), "session-a").expect("proposal should load"),
        );
        let epoch = Utc
            .timestamp_opt(0, 0)
            .single()
            .expect("epoch should be valid");

        assert_eq!(
            (updated, proposals),
            (
                false,
                vec![AgentProposal {
                    session_id: "session-a".to_string(),
                    seq: 1,
                    body: "body".to_string(),
                    created_at: epoch,
                    answer: None,
                    resolved_at: Some(epoch),
                }],
            ),
        );
    }

    #[rstest]
    fn deleting_a_session_removes_its_proposals(sessions_dir: TempDir) {
        let session: crate::commands::agent::types::Session =
            serde_json::from_value(serde_json::json!({
                "session_id": "session-a",
                "cwd": "/fixture/worktree",
                "transcript_path": null,
                "tmux_info": null,
                "status": "stopped",
                "created_at": "2026-03-14T09:26:53Z",
                "updated_at": "2026-03-14T09:26:53Z",
                "last_message": null,
            }))
            .expect("test session should deserialize");
        crate::commands::agent::store::save_session_to(sessions_dir.path(), &session)
            .expect("session should save");
        add_agent_proposal(sessions_dir.path(), "session-a", "body").expect("proposal should save");

        crate::commands::agent::store::delete_session_from(sessions_dir.path(), "session-a")
            .expect("session should delete");

        assert_eq!(
            (
                agent_proposals(sessions_dir.path(), "session-a").expect("proposals should load"),
                unanswered_agent_proposal_bodies(sessions_dir.path(), &["session-a".to_string()],)
                    .expect("unanswered proposal bodies should load"),
            ),
            (Vec::new(), Vec::new()),
        );
    }
}
