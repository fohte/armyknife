use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::open_connection;

/// Checks peer reports after the previous completed Stop decision and advances
/// the cursor in the same transaction so delayed rows cannot be skipped.
pub(crate) fn check_unreported_stop(
    sessions_dir: &Path,
    session_id: &str,
    parent_session_id: &str,
    session_created_at: DateTime<Utc>,
) -> Result<bool> {
    super::validate_session_id(session_id)?;
    super::validate_session_id(parent_session_id)?;

    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let previous_message_id = match transaction
        .query_row(
            "SELECT last_message_id FROM agent_unreported_stop_peer_cursor WHERE session_id = ?1",
            [session_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        Some(last_message_id) => last_message_id,
        None => last_message_id_through(
            &transaction,
            session_id,
            parent_session_id,
            session_created_at,
        )?,
    };
    let latest_parent_message_id = transaction.query_row(
        "SELECT COALESCE(MAX(id), 0) FROM watch_web_peer_out_message
         WHERE from_session_id = ?1 AND to_session_id = ?2",
        params![parent_session_id, session_id],
        |row| row.get::<_, i64>(0),
    )?;
    let report_boundary = previous_message_id.max(latest_parent_message_id);
    let has_reported = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM watch_web_peer_out_message
             WHERE from_session_id = ?1 AND to_session_id = ?2 AND id > ?3
         )",
        params![session_id, parent_session_id, report_boundary],
        |row| row.get::<_, bool>(0),
    )?;
    let last_message_id = transaction.query_row(
        "SELECT COALESCE(MAX(id), 0) FROM watch_web_peer_out_message
         WHERE from_session_id = ?1 AND to_session_id = ?2",
        params![session_id, parent_session_id],
        |row| row.get::<_, i64>(0),
    )?;

    transaction.execute(
        "INSERT INTO agent_unreported_stop_peer_cursor (session_id, last_message_id)
         VALUES (?1, ?2)
         ON CONFLICT(session_id) DO UPDATE SET last_message_id = excluded.last_message_id",
        params![session_id, last_message_id],
    )?;
    transaction.commit()?;
    Ok(has_reported)
}

fn last_message_id_through(
    transaction: &Transaction<'_>,
    session_id: &str,
    parent_session_id: &str,
    boundary: DateTime<Utc>,
) -> Result<i64> {
    let mut statement = transaction.prepare(
        "SELECT id, sent_at FROM watch_web_peer_out_message
         WHERE from_session_id = ?1 AND to_session_id = ?2",
    )?;
    let rows = statement.query_map(params![session_id, parent_session_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut last_message_id = 0;
    for row in rows {
        let (message_id, sent_at) = row?;
        let sent_at = DateTime::parse_from_rfc3339(&sent_at)?.with_timezone(&Utc);
        if sent_at <= boundary {
            last_message_id = last_message_id.max(message_id);
        }
    }
    Ok(last_message_id)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;
    use crate::commands::agent::store::insert_watch_web_peer_out_message_from;

    #[rstest]
    fn counts_late_records_and_ignores_previous_turn_reports(sessions_dir: TempDir) {
        let session_id = "fixture-delegate";
        let parent_id = "fixture-parent";
        let session_created_at = Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap();
        let initial_result = check_unreported_stop(
            sessions_dir.path(),
            session_id,
            parent_id,
            session_created_at,
        )
        .unwrap();
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            session_id,
            parent_id,
            "A report from an earlier turn.",
            Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap(),
            false,
        )
        .unwrap();
        let report_result = check_unreported_stop(
            sessions_dir.path(),
            session_id,
            parent_id,
            session_created_at,
        )
        .unwrap();
        let previous_turn_result = check_unreported_stop(
            sessions_dir.path(),
            session_id,
            parent_id,
            session_created_at,
        )
        .unwrap();

        assert_eq!(
            (initial_result, report_result, previous_turn_result),
            (false, true, false),
        );
    }

    #[rstest]
    fn counts_a_report_sent_after_session_creation_before_the_initial_check(sessions_dir: TempDir) {
        let session_id = "fixture-delegate";
        let parent_id = "fixture-parent";
        let session_created_at = Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap();
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            session_id,
            parent_id,
            "A report before the first check.",
            Utc.with_ymd_and_hms(2026, 6, 1, 11, 0, 0).unwrap(),
            false,
        )
        .unwrap();

        let initial_result = check_unreported_stop(
            sessions_dir.path(),
            session_id,
            parent_id,
            session_created_at,
        )
        .unwrap();
        let next_result = check_unreported_stop(
            sessions_dir.path(),
            session_id,
            parent_id,
            session_created_at,
        )
        .unwrap();

        assert_eq!((initial_result, next_result), (true, false));
    }

    #[rstest]
    fn ignores_reports_before_session_creation_on_the_initial_check(sessions_dir: TempDir) {
        let session_id = "fixture-delegate";
        let parent_id = "fixture-parent";
        let session_created_at = Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap();
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            session_id,
            parent_id,
            "A fictional report from an earlier turn.",
            Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap(),
            false,
        )
        .unwrap();
        let initial_result = check_unreported_stop(
            sessions_dir.path(),
            session_id,
            parent_id,
            session_created_at,
        )
        .unwrap();
        let next_result = check_unreported_stop(
            sessions_dir.path(),
            session_id,
            parent_id,
            session_created_at,
        )
        .unwrap();

        assert_eq!((initial_result, next_result), (false, false));
    }

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }
}
