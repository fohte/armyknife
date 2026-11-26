use std::path::Path;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::open_connection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchWebPeerOutMessage {
    pub(crate) id: i64,
    pub(crate) from_session_id: String,
    pub(crate) to_session_id: String,
    pub(crate) content: String,
    pub(crate) sent_at: DateTime<Utc>,
    pub(crate) failed: bool,
}

pub(crate) fn insert_watch_web_peer_out_message(
    sessions_dir: &Path,
    from_session_id: &str,
    to_session_id: &str,
    content: &str,
    sent_at: DateTime<Utc>,
    failed: bool,
) -> Result<i64> {
    super::validate_session_id(from_session_id)?;
    super::validate_session_id(to_session_id)?;
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
        "INSERT INTO watch_web_peer_out_message
            (from_session_id, to_session_id, content, sent_at, failed)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            from_session_id,
            to_session_id,
            content,
            sent_at.to_rfc3339(),
            failed
        ],
    )?;
    let id = transaction.last_insert_rowid();
    bump_peer_sender_revision(&transaction, from_session_id)?;
    transaction.commit()?;
    Ok(id)
}

pub(crate) fn mark_watch_web_peer_out_message_delivered(
    sessions_dir: &Path,
    id: i64,
    sent_at: DateTime<Utc>,
) -> Result<()> {
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let from_session_id = transaction
        .query_row(
            "SELECT from_session_id FROM watch_web_peer_out_message WHERE id = ?1",
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("peer message {id} does not exist"))?;
    let updated_rows = transaction.execute(
        "UPDATE watch_web_peer_out_message
         SET sent_at = ?1, failed = 0
         WHERE id = ?2",
        params![sent_at.to_rfc3339(), id],
    )?;
    if updated_rows == 0 {
        bail!("peer message {id} does not exist");
    }
    bump_peer_sender_revision(&transaction, &from_session_id)?;
    transaction.commit()?;
    Ok(())
}

fn bump_peer_sender_revision(transaction: &Transaction<'_>, session_id: &str) -> Result<()> {
    let session_exists = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
        [session_id],
        |row| row.get::<_, bool>(0),
    )?;
    if session_exists {
        super::bump_session_revision(transaction, session_id)?;
    } else {
        super::increment_revision(transaction)?;
    }
    Ok(())
}

pub(crate) fn watch_web_peer_out_messages(
    sessions_dir: &Path,
    from_session_id: &str,
) -> Result<Vec<WatchWebPeerOutMessage>> {
    super::validate_session_id(from_session_id)?;
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare(
        "SELECT id, from_session_id, to_session_id, content, sent_at, failed
         FROM watch_web_peer_out_message
         WHERE from_session_id = ?1
         ORDER BY sent_at, id",
    )?;
    let rows = statement.query_map([from_session_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, bool>(5)?,
        ))
    })?;

    rows.map(|row| {
        let (id, from_session_id, to_session_id, content, sent_at, failed) = row?;
        Ok(WatchWebPeerOutMessage {
            id,
            from_session_id,
            to_session_id,
            content,
            sent_at: DateTime::parse_from_rfc3339(&sent_at)?.with_timezone(&Utc),
            failed,
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use crate::commands::agent::store::database::{
        session_revisions_by_session, session_store_revision,
    };

    use super::{
        WatchWebPeerOutMessage, insert_watch_web_peer_out_message,
        mark_watch_web_peer_out_message_delivered, watch_web_peer_out_messages,
    };

    #[fixture]
    fn sessions_dir() -> TempDir {
        let sessions_dir =
            tempfile::tempdir().expect("temporary sessions directory should be created");
        let connection = super::super::open_connection(sessions_dir.path())
            .expect("temporary sessions database should be created");
        for session_id in ["fixture-sender", "fixture-other-sender"] {
            let data = serde_json::json!({
                "session_id": session_id,
                "cwd": "/fixture/worktree",
                "transcript_path": null,
                "tmux_info": null,
                "status": "stopped",
                "created_at": "2026-06-01T10:00:00Z",
                "updated_at": "2026-06-01T10:00:00Z",
                "last_message": null,
            })
            .to_string();
            connection
                .execute(
                    "INSERT INTO sessions (session_id, data, rev) VALUES (?1, ?2, 0)",
                    [session_id, data.as_str()],
                )
                .expect("fixture session should be inserted");
        }
        sessions_dir
    }

    #[rstest]
    fn stores_delivered_messages_with_sender_recipient_content_and_send_time(
        sessions_dir: TempDir,
    ) {
        let later_sent_at = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 12, 30, 0).unwrap();
        let earlier_sent_at = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 12, 29, 0).unwrap();
        let later_id = insert_watch_web_peer_out_message(
            sessions_dir.path(),
            "fixture-sender",
            "fixture-recipient",
            "Delivered after expansion",
            later_sent_at,
            false,
        )
        .unwrap();
        let earlier_id = insert_watch_web_peer_out_message(
            sessions_dir.path(),
            "fixture-sender",
            "fixture-other-recipient",
            "Earlier delivered message",
            earlier_sent_at,
            false,
        )
        .unwrap();
        insert_watch_web_peer_out_message(
            sessions_dir.path(),
            "fixture-other-sender",
            "fixture-recipient",
            "Another sender's message",
            earlier_sent_at,
            false,
        )
        .unwrap();

        assert_eq!(
            (
                watch_web_peer_out_messages(sessions_dir.path(), "fixture-sender").unwrap(),
                session_revisions_by_session(sessions_dir.path()).unwrap(),
                session_store_revision(sessions_dir.path()).unwrap(),
            ),
            (
                vec![
                    WatchWebPeerOutMessage {
                        id: earlier_id,
                        from_session_id: "fixture-sender".to_string(),
                        to_session_id: "fixture-other-recipient".to_string(),
                        content: "Earlier delivered message".to_string(),
                        sent_at: earlier_sent_at,
                        failed: false,
                    },
                    WatchWebPeerOutMessage {
                        id: later_id,
                        from_session_id: "fixture-sender".to_string(),
                        to_session_id: "fixture-recipient".to_string(),
                        content: "Delivered after expansion".to_string(),
                        sent_at: later_sent_at,
                        failed: false,
                    },
                ],
                std::collections::HashMap::from([
                    ("fixture-sender".to_string(), 2),
                    ("fixture-other-sender".to_string(), 3),
                ]),
                3,
            ),
        );
    }

    #[rstest]
    fn stores_failed_delivery_results(sessions_dir: TempDir) {
        let sent_at = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 12, 29, 0).unwrap();
        insert_watch_web_peer_out_message(
            sessions_dir.path(),
            "fixture-sender",
            "fixture-recipient",
            "Delivery failed for the fictional request.",
            sent_at,
            true,
        )
        .unwrap();

        assert_eq!(
            watch_web_peer_out_messages(sessions_dir.path(), "fixture-sender").unwrap(),
            vec![WatchWebPeerOutMessage {
                id: 1,
                from_session_id: "fixture-sender".to_string(),
                to_session_id: "fixture-recipient".to_string(),
                content: "Delivery failed for the fictional request.".to_string(),
                sent_at,
                failed: true,
            }],
        );
    }

    #[rstest]
    fn marks_a_failed_delivery_as_sent_after_redelivery(sessions_dir: TempDir) {
        let failed_at = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 12, 29, 0).unwrap();
        let sent_at = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 12, 30, 0).unwrap();
        let id = insert_watch_web_peer_out_message(
            sessions_dir.path(),
            "fixture-sender",
            "fixture-recipient",
            "A fictional peer message.",
            failed_at,
            true,
        )
        .unwrap();

        mark_watch_web_peer_out_message_delivered(sessions_dir.path(), id, sent_at).unwrap();

        assert_eq!(
            (
                watch_web_peer_out_messages(sessions_dir.path(), "fixture-sender").unwrap(),
                session_revisions_by_session(sessions_dir.path()).unwrap(),
                session_store_revision(sessions_dir.path()).unwrap(),
            ),
            (
                vec![WatchWebPeerOutMessage {
                    id,
                    from_session_id: "fixture-sender".to_string(),
                    to_session_id: "fixture-recipient".to_string(),
                    content: "A fictional peer message.".to_string(),
                    sent_at,
                    failed: false,
                }],
                std::collections::HashMap::from([
                    ("fixture-sender".to_string(), 2),
                    ("fixture-other-sender".to_string(), 0),
                ]),
                2,
            ),
        );
    }

    #[rstest]
    fn records_peer_messages_when_sender_session_is_not_stored(sessions_dir: TempDir) {
        let failed_at = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 12, 29, 0).unwrap();
        let sent_at = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 12, 30, 0).unwrap();
        let id = insert_watch_web_peer_out_message(
            sessions_dir.path(),
            "fixture-untracked-sender",
            "fixture-recipient",
            "A fictional peer message.",
            failed_at,
            true,
        )
        .unwrap();

        mark_watch_web_peer_out_message_delivered(sessions_dir.path(), id, sent_at).unwrap();

        assert_eq!(
            (
                watch_web_peer_out_messages(sessions_dir.path(), "fixture-untracked-sender")
                    .unwrap(),
                session_revisions_by_session(sessions_dir.path()).unwrap(),
                session_store_revision(sessions_dir.path()).unwrap(),
            ),
            (
                vec![WatchWebPeerOutMessage {
                    id,
                    from_session_id: "fixture-untracked-sender".to_string(),
                    to_session_id: "fixture-recipient".to_string(),
                    content: "A fictional peer message.".to_string(),
                    sent_at,
                    failed: false,
                }],
                std::collections::HashMap::from([
                    ("fixture-sender".to_string(), 0),
                    ("fixture-other-sender".to_string(), 0),
                ]),
                2,
            ),
        );
    }
}
