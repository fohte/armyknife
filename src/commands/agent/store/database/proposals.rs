use std::fs::{self, File, OpenOptions};
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{TransactionBehavior, params, params_from_iter};
use sha2::{Digest, Sha256};

use super::{increment_revision, open_connection, validate_session_id};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentProposal {
    pub(crate) session_id: String,
    pub(crate) seq: i64,
    pub(crate) body: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) answer: Option<String>,
    pub(crate) resolved_at: Option<DateTime<Utc>>,
    pub(crate) resolution: Option<ProposalResolution>,
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
    let lock_dir = sessions_dir.join("proposal-resolution-locks");
    fs::create_dir_all(&lock_dir).with_context(|| {
        format!(
            "creating proposal resolution lock directory {}",
            lock_dir.display()
        )
    })?;
    let lock_key = format!("{session_id}:{seq}");
    let lock_hash = Sha256::digest(lock_key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("");
    let lock_path = lock_dir.join(format!("{lock_hash}.lock"));
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening proposal resolution lock {}", lock_path.display()))?;
    lock_file.lock().context("locking proposal resolution")?;
    Ok(lock_file)
}

pub(crate) fn add_agent_proposal(sessions_dir: &Path, session_id: &str, body: &str) -> Result<i64> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
    increment_revision(&transaction)?;
    transaction.commit()?;
    Ok(seq)
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
        "SELECT session_id, seq, body, created_at, answer, resolved_at, resolution, comment
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
        ))
    })?;
    rows.map(|row| {
        let (session_id, seq, body, created_at, answer, resolved_at, resolution, comment) = row?;
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
            comment,
        })
    })
    .collect()
}

pub(crate) fn resolve_agent_proposal(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    comment: Option<&str>,
) -> Result<bool> {
    validate_session_id(session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE agent_proposal
         SET resolution = ?1, comment = ?2, resolved_at = ?3
         WHERE session_id = ?4 AND seq = ?5 AND answer IS NULL AND resolved_at IS NULL",
        params![
            ProposalResolution::Withdrawn.as_str(),
            comment,
            Utc::now().to_rfc3339(),
            session_id,
            seq
        ],
    )?;
    if changed > 0 {
        increment_revision(&transaction)?;
    }
    transaction.commit()?;
    Ok(changed > 0)
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
        "UPDATE agent_proposal SET answer = ?1, resolved_at = COALESCE(resolved_at, ?2)
         WHERE session_id = ?3 AND seq = ?4 AND answer IS NULL AND resolved_at IS NULL",
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
    use rusqlite::{Connection, params};
    use tempfile::TempDir;

    use super::{
        AgentProposal, add_agent_proposal, agent_proposals, unanswered_agent_proposal_bodies,
    };
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
