use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{TransactionBehavior, params};

use super::open_connection;
use crate::commands::agent::types::{Session, SessionStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeferredStop {
    pub(crate) session_id: String,
    pub(crate) parent_session_id: String,
    pub(crate) stopped_at: DateTime<Utc>,
    pub(crate) last_message: Option<String>,
}

/// Replaces the latest pending Stop record only when its reporting turn is
/// still unreported. A reported Stop removes any older deferred record.
pub(crate) fn record_deferred_stop(
    sessions_dir: &Path,
    session: &Session,
    parent_session_id: &str,
    stopped_at: DateTime<Utc>,
) -> Result<bool> {
    super::validate_session_id(&session.session_id)?;
    super::validate_session_id(parent_session_id)?;

    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let has_reported = super::unreported_stop_checks::has_reported_since_last_stop(
        &transaction,
        &session.session_id,
        parent_session_id,
        session.created_at,
    )?;
    if has_reported {
        transaction.execute(
            "DELETE FROM agent_deferred_unreported_stop WHERE session_id = ?1",
            [&session.session_id],
        )?;
    } else {
        transaction.execute(
            "INSERT INTO agent_deferred_unreported_stop (session_id, parent_session_id, stopped_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET
                 parent_session_id = excluded.parent_session_id,
                 stopped_at = excluded.stopped_at",
            params![
                session.session_id,
                parent_session_id,
                stopped_at.to_rfc3339()
            ],
        )?;
    }
    transaction.commit()?;
    Ok(has_reported)
}

pub(crate) fn clear_deferred_stop(sessions_dir: &Path, session_id: &str) -> Result<()> {
    super::validate_session_id(session_id)?;
    let connection = open_connection(sessions_dir)?;
    connection.execute(
        "DELETE FROM agent_deferred_unreported_stop WHERE session_id = ?1",
        [session_id],
    )?;
    Ok(())
}

/// Consumes mature records in one write transaction so overlapping sweeps
/// cannot send the same notification twice.
pub(crate) fn take_mature_deferred_stops(
    sessions_dir: &Path,
    now: DateTime<Utc>,
    quiet_for: Duration,
) -> Result<Vec<DeferredStop>> {
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut statement = transaction.prepare(
        "SELECT deferred.session_id, deferred.parent_session_id, deferred.stopped_at,
                sessions.data,
                (SELECT MAX(sent_at) FROM watch_web_peer_out_message
                 WHERE from_session_id = deferred.session_id
                   AND to_session_id = deferred.parent_session_id) AS last_message_at
         FROM agent_deferred_unreported_stop AS deferred
         LEFT JOIN sessions ON sessions.session_id = deferred.session_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut matured = Vec::new();
    for row in rows {
        let (session_id, parent_session_id, stopped_at, session_data, last_message_at) = row?;
        let stopped_at = DateTime::parse_from_rfc3339(&stopped_at)?.with_timezone(&Utc);
        let last_message_at = last_message_at
            .map(|value| DateTime::parse_from_rfc3339(&value).map(|time| time.with_timezone(&Utc)))
            .transpose()?;
        let last_activity = last_message_at.map_or(stopped_at, |time| time.max(stopped_at));
        if now.signed_duration_since(last_activity) < chrono::Duration::from_std(quiet_for)? {
            continue;
        }

        let session = session_data
            .as_deref()
            .and_then(|data| super::decode_session(&session_id, data));
        let still_stopped_delegate = session.as_ref().is_some_and(|session| {
            let still_stopped = matches!(
                session.status,
                SessionStatus::Stopped | SessionStatus::Paused
            );
            still_stopped
                && session.child_launch
                    == crate::commands::agent::types::ChildLaunchPermission::Deny
                && session.ancestor_session_ids.last() == Some(&parent_session_id)
        });
        transaction.execute(
            "DELETE FROM agent_deferred_unreported_stop WHERE session_id = ?1",
            [&session_id],
        )?;
        if still_stopped_delegate {
            matured.push(DeferredStop {
                session_id,
                parent_session_id,
                stopped_at,
                last_message: session.and_then(|session| session.last_message),
            });
        }
    }
    drop(statement);
    transaction.commit()?;
    Ok(matured)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use rstest::fixture;
    use tempfile::TempDir;

    use super::*;
    use crate::commands::agent::store::{
        check_unreported_stop, insert_watch_web_peer_out_message_from, save_session_to,
    };
    use crate::commands::agent::test_support::deferred_stop_delegate;

    #[fixture]
    fn sessions_dir() -> TempDir {
        TempDir::new().expect("temporary directory should be created")
    }

    fn delegate_session() -> Session {
        deferred_stop_delegate()
    }

    #[rstest::rstest]
    fn records_unreported_stop_without_advancing_the_cursor(sessions_dir: TempDir) {
        let session = delegate_session();
        let stopped_at = Utc.with_ymd_and_hms(2026, 10, 1, 10, 0, 0).unwrap();

        assert!(
            !record_deferred_stop(sessions_dir.path(), &session, "fixture-parent", stopped_at,)
                .unwrap()
        );
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            &session.session_id,
            "fixture-parent",
            "A fictional report.",
            stopped_at + chrono::Duration::minutes(1),
            false,
        )
        .unwrap();
        let first_check = check_unreported_stop(
            sessions_dir.path(),
            &session.session_id,
            "fixture-parent",
            session.created_at,
        )
        .unwrap();

        assert!(first_check);
    }

    #[rstest::rstest]
    fn does_not_record_a_deferred_stop_after_a_report_in_the_same_turn(sessions_dir: TempDir) {
        let session = delegate_session();
        let stopped_at = Utc.with_ymd_and_hms(2026, 10, 1, 10, 0, 0).unwrap();
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            &session.session_id,
            "fixture-parent",
            "A fictional report before the Stop.",
            stopped_at - chrono::Duration::minutes(1),
            false,
        )
        .unwrap();

        let has_reported =
            record_deferred_stop(sessions_dir.path(), &session, "fixture-parent", stopped_at)
                .unwrap();
        let mature = take_mature_deferred_stops(
            sessions_dir.path(),
            stopped_at + chrono::Duration::hours(1),
            Duration::from_secs(30 * 60),
        )
        .unwrap();

        assert_eq!((has_reported, mature), (true, Vec::new()));
    }

    #[rstest::rstest]
    fn reported_stop_removes_deferred_record(sessions_dir: TempDir) {
        let session = delegate_session();
        let stopped_at = Utc.with_ymd_and_hms(2026, 10, 1, 10, 0, 0).unwrap();
        record_deferred_stop(sessions_dir.path(), &session, "fixture-parent", stopped_at).unwrap();
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            &session.session_id,
            "fixture-parent",
            "A fictional report.",
            stopped_at + chrono::Duration::minutes(1),
            false,
        )
        .unwrap();

        assert!(
            record_deferred_stop(
                sessions_dir.path(),
                &session,
                "fixture-parent",
                stopped_at + chrono::Duration::minutes(2),
            )
            .unwrap()
        );
        let matured = take_mature_deferred_stops(
            sessions_dir.path(),
            stopped_at + chrono::Duration::hours(1),
            Duration::from_secs(60),
        )
        .unwrap();

        assert_eq!(matured, Vec::new());
    }

    #[rstest::rstest]
    fn replaces_an_older_pending_stop_time(sessions_dir: TempDir) {
        let session = delegate_session();
        let first_stop = Utc.with_ymd_and_hms(2026, 10, 1, 10, 0, 0).unwrap();
        let latest_stop = first_stop + chrono::Duration::minutes(10);
        save_session_to(sessions_dir.path(), &session).unwrap();
        record_deferred_stop(sessions_dir.path(), &session, "fixture-parent", first_stop).unwrap();
        record_deferred_stop(sessions_dir.path(), &session, "fixture-parent", latest_stop).unwrap();

        let before_quiet_period = take_mature_deferred_stops(
            sessions_dir.path(),
            first_stop + chrono::Duration::minutes(39),
            Duration::from_secs(30 * 60),
        )
        .unwrap();
        let after_quiet_period = take_mature_deferred_stops(
            sessions_dir.path(),
            first_stop + chrono::Duration::minutes(40),
            Duration::from_secs(30 * 60),
        )
        .unwrap();

        assert_eq!(
            (before_quiet_period, after_quiet_period),
            (
                Vec::new(),
                vec![DeferredStop {
                    session_id: "fixture-delegate".to_string(),
                    parent_session_id: "fixture-parent".to_string(),
                    stopped_at: latest_stop,
                    last_message: None,
                }],
            ),
        );
    }

    #[rstest::rstest]
    fn holds_quiet_period_from_latest_peer_activity(sessions_dir: TempDir) {
        let session = delegate_session();
        save_session_to(sessions_dir.path(), &session).unwrap();
        let stopped_at = Utc.with_ymd_and_hms(2026, 10, 1, 10, 0, 0).unwrap();
        record_deferred_stop(sessions_dir.path(), &session, "fixture-parent", stopped_at).unwrap();
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            &session.session_id,
            "fixture-parent",
            "A fictional earlier progress update.",
            stopped_at + chrono::Duration::minutes(5),
            false,
        )
        .unwrap();
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            &session.session_id,
            "fixture-parent",
            "A fictional progress update.",
            stopped_at + chrono::Duration::minutes(25),
            false,
        )
        .unwrap();

        let before_quiet_period = take_mature_deferred_stops(
            sessions_dir.path(),
            stopped_at + chrono::Duration::minutes(54),
            Duration::from_secs(30 * 60),
        )
        .unwrap();
        let after_quiet_period = take_mature_deferred_stops(
            sessions_dir.path(),
            stopped_at + chrono::Duration::minutes(55),
            Duration::from_secs(30 * 60),
        )
        .unwrap();
        assert_eq!(
            (before_quiet_period, after_quiet_period),
            (
                Vec::new(),
                vec![DeferredStop {
                    session_id: "fixture-delegate".to_string(),
                    parent_session_id: "fixture-parent".to_string(),
                    stopped_at,
                    last_message: None,
                }],
            ),
        );
    }
}
