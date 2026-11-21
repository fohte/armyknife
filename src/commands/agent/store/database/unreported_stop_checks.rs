use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::open_connection;

/// Checks peer reports after the previous completed Stop decision and advances
/// the boundary in the same transaction so concurrent Stop hooks cannot race.
pub(crate) fn check_unreported_stop(
    sessions_dir: &Path,
    session_id: &str,
    parent_session_id: &str,
    session_created_at: DateTime<Utc>,
    checked_at: DateTime<Utc>,
) -> Result<bool> {
    super::validate_session_id(session_id)?;
    super::validate_session_id(parent_session_id)?;

    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let previous_check = transaction
        .query_row(
            "SELECT checked_at FROM agent_unreported_stop_check WHERE session_id = ?1",
            [session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let boundary = previous_check
        .map(|value| DateTime::parse_from_rfc3339(&value).map(|value| value.with_timezone(&Utc)))
        .transpose()?
        .unwrap_or(session_created_at);

    let mut statement = transaction.prepare(
        "SELECT sent_at FROM watch_web_peer_out_message
         WHERE from_session_id = ?1 AND to_session_id = ?2",
    )?;
    let rows = statement.query_map(params![session_id, parent_session_id], |row| {
        row.get::<_, String>(0)
    })?;
    let mut reported = false;
    for row in rows {
        let sent_at = DateTime::parse_from_rfc3339(&row?)?.with_timezone(&Utc);
        if sent_at > boundary && sent_at <= checked_at {
            reported = true;
            break;
        }
    }
    drop(statement);

    transaction.execute(
        "INSERT INTO agent_unreported_stop_check (session_id, checked_at)
         VALUES (?1, ?2)
         ON CONFLICT(session_id) DO UPDATE SET checked_at = excluded.checked_at",
        params![session_id, checked_at.to_rfc3339()],
    )?;
    transaction.commit()?;
    Ok(reported)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;
    use crate::commands::agent::store::insert_watch_web_peer_out_message_from;

    #[rstest]
    #[case::before_boundary("2026-06-01T12:00:00Z", false)]
    #[case::at_boundary("2026-06-01T12:01:00Z", false)]
    #[case::after_boundary("2026-06-01T12:01:01Z", true)]
    fn detects_only_parent_reports_after_the_previous_check(
        #[case] sent_at: &str,
        #[case] expected: bool,
        sessions_dir: TempDir,
    ) {
        let session_id = "fixture-delegate";
        let parent_id = "fixture-parent";
        let sent_at = DateTime::parse_from_rfc3339(sent_at)
            .unwrap()
            .with_timezone(&Utc);
        check_unreported_stop(
            sessions_dir.path(),
            session_id,
            parent_id,
            Utc.with_ymd_and_hms(2026, 6, 1, 11, 59, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 6, 1, 12, 1, 0).unwrap(),
        )
        .unwrap();
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            session_id,
            parent_id,
            "A fictional report.",
            sent_at,
            true,
        )
        .unwrap();

        assert_eq!(
            check_unreported_stop(
                sessions_dir.path(),
                session_id,
                parent_id,
                Utc.with_ymd_and_hms(2026, 6, 1, 11, 59, 0).unwrap(),
                Utc.with_ymd_and_hms(2026, 6, 1, 12, 2, 0).unwrap(),
            )
            .unwrap(),
            expected,
        );
    }

    #[rstest]
    fn advances_the_boundary_after_each_completed_decision(sessions_dir: TempDir) {
        let session_id = "fixture-delegate";
        let parent_id = "fixture-parent";
        let session_created_at = Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap();
        let first_check = Utc.with_ymd_and_hms(2026, 6, 1, 11, 30, 0).unwrap();
        let second_check = Utc.with_ymd_and_hms(2026, 6, 1, 12, 1, 0).unwrap();
        let third_check = Utc.with_ymd_and_hms(2026, 6, 1, 12, 2, 0).unwrap();
        let initial_result = check_unreported_stop(
            sessions_dir.path(),
            session_id,
            parent_id,
            session_created_at,
            first_check,
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
            second_check,
        )
        .unwrap();
        let previous_turn_result = check_unreported_stop(
            sessions_dir.path(),
            session_id,
            parent_id,
            session_created_at,
            third_check,
        )
        .unwrap();

        assert_eq!(
            (initial_result, report_result, previous_turn_result),
            (false, true, false),
        );
    }

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }
}
