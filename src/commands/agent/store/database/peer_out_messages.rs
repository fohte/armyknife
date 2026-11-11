use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::params;

use super::open_connection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchWebPeerOutMessage {
    pub(crate) id: i64,
    pub(crate) from_session_id: String,
    pub(crate) to_session_id: String,
    pub(crate) content: String,
    pub(crate) sent_at: DateTime<Utc>,
}

pub(crate) fn insert_watch_web_peer_out_message(
    sessions_dir: &Path,
    from_session_id: &str,
    to_session_id: &str,
    content: &str,
    sent_at: DateTime<Utc>,
) -> Result<i64> {
    super::validate_session_id(from_session_id)?;
    super::validate_session_id(to_session_id)?;
    let connection = open_connection(sessions_dir)?;
    connection.execute(
        "INSERT INTO watch_web_peer_out_message
            (from_session_id, to_session_id, content, sent_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            from_session_id,
            to_session_id,
            content,
            sent_at.to_rfc3339()
        ],
    )?;
    Ok(connection.last_insert_rowid())
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
        "SELECT id, from_session_id, to_session_id, content, sent_at
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
        ))
    })?;

    rows.map(|row| {
        let (id, from_session_id, to_session_id, content, sent_at) = row?;
        Ok(WatchWebPeerOutMessage {
            id,
            from_session_id,
            to_session_id,
            content,
            sent_at: DateTime::parse_from_rfc3339(&sent_at)?.with_timezone(&Utc),
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::{
        WatchWebPeerOutMessage, insert_watch_web_peer_out_message, watch_web_peer_out_messages,
    };

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().expect("temporary sessions directory should be created")
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
        )
        .unwrap();
        let earlier_id = insert_watch_web_peer_out_message(
            sessions_dir.path(),
            "fixture-sender",
            "fixture-other-recipient",
            "Earlier delivered message",
            earlier_sent_at,
        )
        .unwrap();
        insert_watch_web_peer_out_message(
            sessions_dir.path(),
            "fixture-other-sender",
            "fixture-recipient",
            "Another sender's message",
            earlier_sent_at,
        )
        .unwrap();

        assert_eq!(
            watch_web_peer_out_messages(sessions_dir.path(), "fixture-sender").unwrap(),
            vec![
                WatchWebPeerOutMessage {
                    id: earlier_id,
                    from_session_id: "fixture-sender".to_string(),
                    to_session_id: "fixture-other-recipient".to_string(),
                    content: "Earlier delivered message".to_string(),
                    sent_at: earlier_sent_at,
                },
                WatchWebPeerOutMessage {
                    id: later_id,
                    from_session_id: "fixture-sender".to_string(),
                    to_session_id: "fixture-recipient".to_string(),
                    content: "Delivered after expansion".to_string(),
                    sent_at: later_sent_at,
                },
            ],
        );
    }
}
