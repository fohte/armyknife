use indoc::indoc;
use rstest::rstest;
use rusqlite::Connection;
use rusqlite_migration::{Error as MigrationError, MigrationDefinitionError, Migrations};

use super::*;

#[rstest]
#[case::newer_database(1, true)]
#[case::current_database(0, false)]
#[case::older_database(-1, false)]
fn accepts_database_too_far_ahead_only_for_a_newer_version(
    #[case] version_offset: i64,
    #[case] expected_accepted: bool,
) {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "user_version", current_version() + version_offset)
        .expect("database migration version should be stored");

    let error = MigrationError::MigrationDefinition(MigrationDefinitionError::DatabaseTooFarAhead);
    let actual = accept_migration_error(&connection, error).is_ok();

    assert_eq!(actual, expected_accepted);
}

#[test]
fn adds_peer_out_message_to_version_seven_database() {
    let temp_dir = tempfile::tempdir().expect("temporary directory should be created");
    let database_path = temp_dir.path().join("sessions.db");
    let mut connection = Connection::open(&database_path).expect("temporary database should open");
    Migrations::from_slice(&MIGRATION_ENTRIES[..7])
        .to_latest(&mut connection)
        .expect("master migrations should create the version seven schema");
    let initial_user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("database migration version should be readable");
    connection
        .execute(
            "INSERT INTO agent_decision (
                session_id, seq, title, why, considered, easy_to_change, created_at
            ) VALUES (
                'sample-session', 1, 'sample choice', 'sample reason',
                'sample alternative', 'sample change', '2026-01-01T00:00:00Z'
            )",
            [],
        )
        .expect("version seven decision row should be inserted");
    drop(connection);

    let connection = super::super::open_connection(temp_dir.path())
        .expect("version seven database should migrate to the peer message schema");
    let actual = connection
        .query_row(
            indoc! {
                "
                SELECT
                EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'watch_web_peer_out_message'),
                EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'watch_web_peer_out_message_sender_target_time'),
                (SELECT title || ':' || why FROM agent_decision LIMIT 1)
                "
            },
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                ))
            },
        )
        .expect("peer schema and retained decision row should be queryable");
    let actual_user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("database migration version should be readable");

    assert_eq!(
        (initial_user_version, actual, actual_user_version),
        (
            7,
            (true, true, Some("sample choice:sample reason".to_string())),
            current_version(),
        ),
    );
}
