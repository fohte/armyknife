use std::fs::File;
use std::path::Path;

use super::WithdrawalReason;
use super::resolution_lock::lock_resolution_file;
use super::{bump_session_revision, open_connection, validate_session_id};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{TransactionBehavior, params, params_from_iter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentProposal {
    pub(crate) session_id: String,
    pub(crate) seq: i64,
    pub(crate) body: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) answer: Option<String>,
    pub(crate) resolved_at: Option<DateTime<Utc>>,
    pub(crate) resolution: Option<ProposalResolution>,
    pub(crate) withdrawal_reason: Option<WithdrawalReason>,
    pub(crate) comment: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProposalResolution {
    Withdrawn,
}

impl ProposalResolution {
    fn as_str(self) -> &'static str {
        match self {
            Self::Withdrawn => "withdrawn",
        }
    }

    fn parse(value: Option<String>) -> Result<Option<Self>> {
        match value.as_deref() {
            Some("withdrawn") => Ok(Some(Self::Withdrawn)),
            Some(value) => anyhow::bail!("unknown proposal resolution: {value}"),
            None => Ok(None),
        }
    }
}

pub(crate) fn lock_agent_proposal_resolution(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
) -> Result<File> {
    validate_session_id(session_id)?;
    let lock_key = format!("{session_id}:{seq}");
    lock_resolution_file(sessions_dir, "proposal-resolution-locks", &lock_key)
}

pub(crate) fn add_agent_proposal(sessions_dir: &Path, session_id: &str, body: &str) -> Result<i64> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let session_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
        [session_id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(session_exists, "session not found: {session_id}");
    let now = Utc::now().to_rfc3339();
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
    bump_session_revision(&transaction, session_id)?;
    transaction.commit()?;
    Ok(seq)
}

pub(crate) fn edit_agent_proposal(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    body: &str,
) -> Result<bool> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE agent_proposal SET body = ?1
         WHERE session_id = ?2 AND seq = ?3 AND answer IS NULL AND resolved_at IS NULL",
        params![body, session_id, seq],
    )?;
    if changed > 0 {
        bump_session_revision(&transaction, session_id)?;
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
        "SELECT session_id, seq, body, created_at, answer, resolved_at, resolution, withdrawal_reason, comment
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
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
        ))
    })?;
    rows.map(|row| {
        let (
            session_id,
            seq,
            body,
            created_at,
            answer,
            resolved_at,
            resolution,
            withdrawal_reason,
            comment,
        ) = row?;
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
            resolution: ProposalResolution::parse(resolution)?,
            withdrawal_reason: WithdrawalReason::parse(withdrawal_reason)?,
            comment,
        })
    })
    .collect()
}

#[cfg(test)]
pub(crate) fn resolve_agent_proposal(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    comment: Option<&str>,
) -> Result<bool> {
    resolve_agent_proposal_with_reason(sessions_dir, session_id, seq, comment, None, Utc::now())
}

pub(crate) fn resolve_agent_proposal_with_reason(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    comment: Option<&str>,
    withdrawal_reason: Option<WithdrawalReason>,
    resolved_at: DateTime<Utc>,
) -> Result<bool> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE agent_proposal
         SET resolution = ?1, comment = ?2, resolved_at = ?3, withdrawal_reason = ?4
         WHERE session_id = ?5 AND seq = ?6 AND answer IS NULL AND resolved_at IS NULL",
        params![
            ProposalResolution::Withdrawn.as_str(),
            comment,
            resolved_at.to_rfc3339(),
            withdrawal_reason.map(WithdrawalReason::as_str),
            session_id,
            seq
        ],
    )?;
    if changed > 0 {
        bump_session_revision(&transaction, session_id)?;
    }
    transaction.commit()?;
    Ok(changed > 0)
}

pub(crate) fn write_agent_proposal_answer_at(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    answer: &str,
    resolved_at: DateTime<Utc>,
) -> Result<bool> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE agent_proposal SET answer = ?1, resolved_at = ?2
         WHERE session_id = ?3 AND seq = ?4 AND answer IS NULL AND resolved_at IS NULL",
        params![answer, resolved_at.to_rfc3339(), session_id, seq],
    )?;
    if changed > 0 {
        bump_session_revision(&transaction, session_id)?;
    }
    transaction.commit()?;
    Ok(changed > 0)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};
    use rusqlite::{Connection, params};
    use tempfile::TempDir;

    use super::super::WithdrawalReason;
    use super::{
        AgentProposal, add_agent_proposal, agent_proposals, lock_agent_proposal_resolution,
        resolve_agent_proposal_with_reason, unanswered_agent_proposal_bodies,
        write_agent_proposal_answer_at,
    };
    #[fixture]
    fn sessions_dir() -> TempDir {
        let sessions_dir =
            tempfile::tempdir().expect("temporary sessions directory should be created");
        for session_id in ["session-a", "session-b"] {
            let session = serde_json::from_value(serde_json::json!({
                "session_id": session_id,
                "cwd": "/tmp/fixture-worktree",
                "status": "running",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
            }))
            .expect("test session should deserialize");
            crate::commands::agent::store::save_session_to(sessions_dir.path(), &session)
                .expect("test session should save");
        }
        sessions_dir
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
    #[case::session_ended(WithdrawalReason::SessionEnded)]
    #[case::worktree_cleanup(WithdrawalReason::WorktreeCleanup)]
    #[case::manual(WithdrawalReason::Manual)]
    fn proposal_withdrawal_persists_its_reason(
        sessions_dir: TempDir,
        #[case] reason: WithdrawalReason,
    ) {
        let seq = add_agent_proposal(sessions_dir.path(), "session-a", "Proposal body")
            .expect("proposal should be stored");
        let resolved_at = Utc
            .timestamp_opt(100, 0)
            .single()
            .expect("time should be valid");

        resolve_agent_proposal_with_reason(
            sessions_dir.path(),
            "session-a",
            seq,
            Some("Withdrawal note"),
            Some(reason),
            resolved_at,
        )
        .expect("proposal should be withdrawn");

        let mut actual = agent_proposals(sessions_dir.path(), "session-a")
            .expect("proposal should load")
            .remove(0);
        actual.created_at = Utc
            .timestamp_opt(0, 0)
            .single()
            .expect("epoch should be valid");
        actual.resolved_at = Some(resolved_at);
        assert_eq!(
            actual,
            AgentProposal {
                session_id: "session-a".to_string(),
                seq: 1,
                body: "Proposal body".to_string(),
                created_at: Utc
                    .timestamp_opt(0, 0)
                    .single()
                    .expect("epoch should be valid"),
                answer: None,
                resolved_at: Some(resolved_at),
                resolution: Some(super::ProposalResolution::Withdrawn),
                withdrawal_reason: Some(reason),
                comment: Some("Withdrawal note".to_string()),
            },
        );
    }

    #[rstest]
    fn proposal_resolution_lock_keeps_its_existing_file_name(sessions_dir: TempDir) {
        let _lock = lock_agent_proposal_resolution(sessions_dir.path(), "session-a", 4)
            .expect("proposal resolution lock should be acquired");
        let lock_files = std::fs::read_dir(sessions_dir.path().join("proposal-resolution-locks"))
            .expect("proposal resolution lock directory should exist")
            .map(|entry| {
                entry
                    .expect("proposal resolution lock should be readable")
                    .file_name()
                    .into_string()
                    .expect("proposal resolution lock name should be valid UTF-8")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            lock_files,
            vec!["43f4638657d0f42f33cbdddeacffa1745da54c6f9c6c7390cdbb72234d0e4623.lock"],
        );
    }

    #[rstest]
    fn adds_proposals_in_sequence_without_resolving_previous_unanswered_proposals(
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
                        resolved_at: None,
                        resolution: None,
                        withdrawal_reason: None,
                        comment: None,
                    },
                    AgentProposal {
                        session_id: "session-a".to_string(),
                        seq: 2,
                        body: "second body".to_string(),
                        created_at: epoch,
                        answer: None,
                        resolved_at: None,
                        resolution: None,
                        withdrawal_reason: None,
                        comment: None,
                    },
                ],
                vec![
                    ("session-a".to_string(), "first body".to_string()),
                    ("session-a".to_string(), "second body".to_string()),
                    ("session-b".to_string(), "other body".to_string()),
                ],
            ),
        );
    }

    #[rstest]
    fn rejects_adding_a_proposal_for_an_untracked_session(sessions_dir: TempDir) {
        let result = add_agent_proposal(sessions_dir.path(), "untracked-session", "body")
            .map_err(|error| error.to_string());
        let proposals = agent_proposals(sessions_dir.path(), "untracked-session")
            .expect("proposals should load");

        assert_eq!(
            (result, proposals),
            (
                Err("session not found: untracked-session".to_string()),
                Vec::new(),
            ),
        );
    }

    #[rstest]
    fn records_the_supplied_answer_resolution_time(sessions_dir: TempDir) {
        add_agent_proposal(sessions_dir.path(), "session-a", "body").expect("proposal should save");
        let resolved_at = Utc
            .with_ymd_and_hms(2026, 6, 1, 12, 0, 1)
            .single()
            .expect("test timestamp should be valid");

        let updated = write_agent_proposal_answer_at(
            sessions_dir.path(),
            "session-a",
            1,
            "{\"choice\":0}",
            resolved_at,
        )
        .expect("answer should save");
        let proposals = agent_proposals(sessions_dir.path(), "session-a")
            .expect("proposal should load")
            .into_iter()
            .map(|proposal| (proposal.seq, proposal.answer, proposal.resolved_at))
            .collect::<Vec<_>>();

        assert_eq!(
            (updated, proposals),
            (
                true,
                vec![(1, Some("{\"choice\":0}".to_string()), Some(resolved_at))],
            ),
        );
    }

    #[rstest]
    fn unanswered_bodies_exclude_resolved_proposals_without_answers(sessions_dir: TempDir) {
        add_agent_proposal(sessions_dir.path(), "session-a", "closed proposal")
            .expect("proposal should save");
        let connection = Connection::open(sessions_dir.path().join("sessions.db"))
            .expect("session database should open");
        connection
            .execute(
                "UPDATE agent_proposal SET resolved_at = ?1
                 WHERE session_id = ?2 AND seq = ?3",
                params!["2026-01-01T00:00:00Z", "session-a", 1],
            )
            .expect("resolution timestamp should save");

        let actual =
            unanswered_agent_proposal_bodies(sessions_dir.path(), &["session-a".to_string()])
                .expect("unanswered proposal bodies should load");

        assert_eq!(actual, Vec::new());
    }

    #[rstest]
    fn deleting_a_session_removes_its_proposals(sessions_dir: TempDir) {
        add_agent_proposal(sessions_dir.path(), "session-a", "body").expect("proposal should save");

        crate::commands::agent::store::delete_session_from_with_storage(
            sessions_dir.path(),
            "session-a",
            Some(&sessions_dir.path().join("attachments")),
        )
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
