use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, ensure};
use chrono::Utc;
use rusqlite::{TransactionBehavior, params};

use super::{increment_revision, open_connection, validate_session_id};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentDecision {
    pub(crate) session_id: String,
    pub(crate) seq: i64,
    pub(crate) title: String,
    pub(crate) why: String,
    pub(crate) considered: Vec<String>,
    pub(crate) easy_to_change: String,
    pub(crate) created_at: String,
    pub(crate) review: Option<String>,
    pub(crate) reviewed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AgentDecisionCounts {
    pub(crate) total: i64,
    pub(crate) unreviewed: i64,
}

pub(crate) fn insert_agent_decision(
    sessions_dir: &Path,
    session_id: &str,
    title: &str,
    why: &str,
    considered: &[String],
    easy_to_change: &str,
) -> Result<i64> {
    validate_session_id(session_id)?;
    let considered = serde_json::to_string(considered)?;
    let created_at = Utc::now().to_rfc3339();
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let session_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
        [session_id],
        |row| row.get(0),
    )?;
    ensure!(session_exists, "session not found: {session_id}");
    let seq: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM agent_decision WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO agent_decision
            (session_id, seq, title, why, considered, easy_to_change, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            session_id,
            seq,
            title,
            why,
            considered,
            easy_to_change,
            created_at
        ],
    )?;
    bump_session_revision(&transaction, session_id)?;
    transaction.commit()?;
    Ok(seq)
}

pub(crate) fn agent_decisions(sessions_dir: &Path, session_id: &str) -> Result<Vec<AgentDecision>> {
    validate_session_id(session_id)?;
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }
    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare(
        "SELECT session_id, seq, title, why, considered, easy_to_change,
                created_at, review, reviewed_at
         FROM agent_decision WHERE session_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map([session_id], |row| {
        let considered: String = row.get(4)?;
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            considered,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
        ))
    })?;
    rows.map(|row| {
        let (
            session_id,
            seq,
            title,
            why,
            considered,
            easy_to_change,
            created_at,
            review,
            reviewed_at,
        ) = row?;
        Ok(AgentDecision {
            session_id,
            seq,
            title,
            why,
            considered: serde_json::from_str(&considered)?,
            easy_to_change,
            created_at,
            review,
            reviewed_at,
        })
    })
    .collect()
}

pub(crate) fn agent_decision_counts_by_session(
    sessions_dir: &Path,
) -> Result<HashMap<String, AgentDecisionCounts>> {
    if !sessions_dir.exists() {
        return Ok(HashMap::new());
    }
    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare(
        "SELECT session_id, COUNT(*),
                COALESCE(SUM(CASE WHEN review IS NULL THEN 1 ELSE 0 END), 0)
         FROM agent_decision
         GROUP BY session_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get(0)?,
            AgentDecisionCounts {
                total: row.get(1)?,
                unreviewed: row.get(2)?,
            },
        ))
    })?;
    rows.collect::<rusqlite::Result<HashMap<_, _>>>()
        .map_err(Into::into)
}

pub(crate) fn set_agent_decision_review(
    sessions_dir: &Path,
    session_id: &str,
    seq: i64,
    review: &str,
) -> Result<()> {
    validate_session_id(session_id)?;
    ensure!(
        matches!(review, "ok" | "change"),
        "unknown decision review `{review}`; expected `ok` or `change`"
    );
    let reviewed_at = Utc::now().to_rfc3339();
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let updated = transaction.execute(
        "UPDATE agent_decision SET review = ?1, reviewed_at = ?2
         WHERE session_id = ?3 AND seq = ?4",
        params![review, reviewed_at, session_id, seq],
    )?;
    ensure!(
        updated == 1,
        "decision {seq} not found for session {session_id}"
    );
    bump_session_revision(&transaction, session_id)?;
    transaction.commit()?;
    Ok(())
}

fn bump_session_revision(transaction: &rusqlite::Transaction<'_>, session_id: &str) -> Result<()> {
    let revision = increment_revision(transaction)?;
    let updated = transaction.execute(
        "UPDATE sessions SET rev = ?1 WHERE session_id = ?2",
        params![revision, session_id],
    )?;
    ensure!(
        updated == 1,
        "session {session_id} disappeared during decision update"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::store::{
        agent_decision_counts_by_session_from, session_store_revision_from,
    };
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    #[fixture]
    fn sessions_dir() -> TempDir {
        TempDir::new().expect("temporary directory should be created")
    }

    fn create_session(sessions_dir: &Path) {
        let connection = open_connection(sessions_dir).expect("database should open");
        connection
            .execute(
                "INSERT INTO sessions (session_id, data, rev) VALUES (?1, '{}', 0)",
                ["session-cobalt"],
            )
            .expect("session row should be stored");
    }

    fn create_additional_session(sessions_dir: &Path, session_id: &str) {
        let connection = open_connection(sessions_dir).expect("database should open");
        connection
            .execute(
                "INSERT INTO sessions (session_id, data, rev) VALUES (?1, '{}', 0)",
                [session_id],
            )
            .expect("session row should be stored");
    }

    fn normalize_decision_timestamps(mut decision: AgentDecision) -> (AgentDecision, bool, bool) {
        let created_at_is_rfc3339 =
            chrono::DateTime::parse_from_rfc3339(&decision.created_at).is_ok();
        let reviewed_at_is_rfc3339 = decision
            .reviewed_at
            .as_deref()
            .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok());
        decision.created_at = "normalized timestamp".to_string();
        decision.reviewed_at = decision
            .reviewed_at
            .map(|_| "normalized timestamp".to_string());
        (decision, created_at_is_rfc3339, reviewed_at_is_rfc3339)
    }

    #[rstest]
    fn insert_assigns_per_session_sequences_and_bumps_revision(sessions_dir: TempDir) {
        create_session(sessions_dir.path());
        let before = session_store_revision_from(sessions_dir.path()).expect("revision loads");
        let considered = vec!["keep current format".to_string()];

        let seqs = [
            insert_agent_decision(
                sessions_dir.path(),
                "session-cobalt",
                "first",
                "reason",
                &considered,
                "easy",
            )
            .expect("first decision is stored"),
            insert_agent_decision(
                sessions_dir.path(),
                "session-cobalt",
                "second",
                "reason",
                &considered,
                "easy",
            )
            .expect("second decision is stored"),
        ];

        assert_eq!(
            (
                seqs,
                session_store_revision_from(sessions_dir.path()).expect("revision loads")
            ),
            ([1, 2], before + 2),
        );
    }

    #[rstest]
    fn insert_fails_for_unknown_sessions(sessions_dir: TempDir) {
        let result = insert_agent_decision(
            sessions_dir.path(),
            "session-cobalt",
            "title",
            "reason",
            &["alternative".to_string()],
            "easy",
        );

        assert_eq!(
            result.unwrap_err().to_string(),
            "session not found: session-cobalt"
        );
    }

    #[rstest]
    fn deleting_a_session_removes_its_decisions(sessions_dir: TempDir) {
        create_session(sessions_dir.path());
        insert_agent_decision(
            sessions_dir.path(),
            "session-cobalt",
            "title",
            "reason",
            &["alternative".to_string()],
            "easy",
        )
        .expect("decision is stored");

        let removed = super::super::delete_session(sessions_dir.path(), "session-cobalt")
            .expect("session should be deleted");
        let decisions =
            agent_decisions(sessions_dir.path(), "session-cobalt").expect("decisions load");
        let counts = decision_counts(sessions_dir.path(), "session-cobalt");

        assert_eq!(
            (removed, decisions, counts),
            (
                true,
                Vec::new(),
                AgentDecisionCounts {
                    total: 0,
                    unreviewed: 0,
                }
            ),
        );
    }

    #[rstest]
    fn listing_counts_and_review_updates_cover_the_saved_decisions(sessions_dir: TempDir) {
        create_session(sessions_dir.path());
        let initial_counts = decision_counts(sessions_dir.path(), "session-cobalt");
        insert_agent_decision(
            sessions_dir.path(),
            "session-cobalt",
            "title",
            "reason",
            &["alternative".to_string()],
            "easy",
        )
        .expect("decision is stored");
        let unreviewed_counts = decision_counts(sessions_dir.path(), "session-cobalt");
        let before_review =
            session_store_revision_from(sessions_dir.path()).expect("revision loads");
        crate::commands::agent::store::set_agent_decision_review_from(
            sessions_dir.path(),
            "session-cobalt",
            1,
            "ok",
        )
        .expect("review is stored");

        let decisions =
            agent_decisions(sessions_dir.path(), "session-cobalt").expect("decisions load");
        let counts = decision_counts(sessions_dir.path(), "session-cobalt");
        let revision = session_store_revision_from(sessions_dir.path()).expect("revision loads");

        assert_eq!(
            (
                decisions
                    .iter()
                    .cloned()
                    .map(normalize_decision_timestamps)
                    .collect::<Vec<_>>(),
                initial_counts,
                unreviewed_counts,
                counts,
                revision,
            ),
            (
                vec![(
                    AgentDecision {
                        session_id: "session-cobalt".to_string(),
                        seq: 1,
                        title: "title".to_string(),
                        why: "reason".to_string(),
                        considered: vec!["alternative".to_string()],
                        easy_to_change: "easy".to_string(),
                        created_at: "normalized timestamp".to_string(),
                        review: Some("ok".to_string()),
                        reviewed_at: Some("normalized timestamp".to_string()),
                    },
                    true,
                    true,
                )],
                AgentDecisionCounts {
                    total: 0,
                    unreviewed: 0
                },
                AgentDecisionCounts {
                    total: 1,
                    unreviewed: 1
                },
                AgentDecisionCounts {
                    total: 1,
                    unreviewed: 0
                },
                before_review + 1,
            ),
        );
    }

    fn decision_counts(sessions_dir: &Path, session_id: &str) -> AgentDecisionCounts {
        agent_decision_counts_by_session_from(sessions_dir)
            .expect("grouped counts load")
            .get(session_id)
            .copied()
            .unwrap_or(AgentDecisionCounts {
                total: 0,
                unreviewed: 0,
            })
    }

    #[rstest]
    fn decision_counts_are_grouped_by_session(sessions_dir: TempDir) {
        create_session(sessions_dir.path());
        create_additional_session(sessions_dir.path(), "session-amber");
        let considered = ["alternative".to_string()];
        insert_agent_decision(
            sessions_dir.path(),
            "session-cobalt",
            "first",
            "reason",
            &considered,
            "easy",
        )
        .expect("first decision is stored");
        insert_agent_decision(
            sessions_dir.path(),
            "session-cobalt",
            "second",
            "reason",
            &considered,
            "easy",
        )
        .expect("second decision is stored");
        insert_agent_decision(
            sessions_dir.path(),
            "session-amber",
            "third",
            "reason",
            &considered,
            "easy",
        )
        .expect("third decision is stored");
        crate::commands::agent::store::set_agent_decision_review_from(
            sessions_dir.path(),
            "session-cobalt",
            1,
            "ok",
        )
        .expect("review is stored");

        let counts = agent_decision_counts_by_session_from(sessions_dir.path())
            .expect("grouped counts load");

        assert_eq!(
            counts,
            HashMap::from([
                (
                    "session-cobalt".to_string(),
                    AgentDecisionCounts {
                        total: 2,
                        unreviewed: 1,
                    },
                ),
                (
                    "session-amber".to_string(),
                    AgentDecisionCounts {
                        total: 1,
                        unreviewed: 1,
                    },
                ),
            ]),
        );
    }
}
