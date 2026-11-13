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
fn adds_permission_request_mailbox_to_version_nine_database() {
    let temp_dir = tempfile::tempdir().expect("temporary directory should be created");
    let database_path = temp_dir.path().join("sessions.db");
    let mut connection = Connection::open(&database_path).expect("temporary database should open");
    Migrations::from_slice(&MIGRATION_ENTRIES[..9])
        .to_latest(&mut connection)
        .expect("master migrations should create the version nine schema");
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
        .expect("version nine decision row should be inserted");
    connection
        .execute(
            "INSERT INTO watch_web_peer_out_message (
                from_session_id, to_session_id, content, sent_at
            ) VALUES (
                'fixture-sender', 'fixture-target', 'A fictional peer message.', '2026-01-01T00:00:00Z'
            )",
            [],
        )
        .expect("version nine peer message should be inserted");
    drop(connection);

    let connection = super::super::open_connection(temp_dir.path())
        .expect("version nine database should migrate to the permission mailbox schema");
    let actual = connection
        .query_row(
            indoc! {
                "
                SELECT
                EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'watch_web_peer_out_message'),
                EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'watch_web_peer_out_message_sender_target_time'),
                EXISTS(SELECT 1 FROM pragma_table_info('watch_web_session_state') WHERE name = 'primary_task_id'),
                EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'permission_request_decision'),
                EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'permission_request_decision_session_agent'),
                EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = 'permission_request_decision_delete_session'),
                EXISTS(SELECT 1 FROM pragma_table_info('permission_request_decision') WHERE name = 'answers' AND [notnull] = 0),
                EXISTS(SELECT 1 FROM pragma_table_info('watch_web_peer_out_message') WHERE name = 'failed' AND [notnull] = 1),
                (SELECT failed FROM watch_web_peer_out_message WHERE from_session_id = 'fixture-sender'),
                (SELECT title || ':' || why FROM agent_decision LIMIT 1)
                "
            },
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
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
            9,
            (
                true,
                true,
                true,
                true,
                true,
                true,
                true,
                true,
                Some(0_i64),
                Some("sample choice:sample reason".to_string())
            ),
            current_version(),
        ),
    );
}
#[test]
fn adds_session_replacement_table_to_version_fifteen_database() {
    let temp_dir = tempfile::tempdir().expect("temporary database directory should be created");
    let database_path = temp_dir.path().join("sessions.db");
    let mut connection = Connection::open(&database_path).expect("temporary database should open");
    Migrations::from_slice(&MIGRATION_ENTRIES[..15])
        .to_latest(&mut connection)
        .expect("version fifteen schema should be created");
    drop(connection);

    let connection = super::super::open_connection(temp_dir.path())
        .expect("version fifteen database should migrate to the latest schema");
    let actual = connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'session_replacement'
             )",
            [],
            |row| row.get::<_, bool>(0),
        )
        .expect("replacement table should be queryable");
    let actual_user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("database migration version should be readable");

    assert_eq!((actual, actual_user_version), (true, current_version()));
}

#[test]
fn upgrades_legacy_rewind_version_fourteen_database() {
    let temp_dir = tempfile::tempdir().expect("temporary database directory should be created");
    let database_path = temp_dir.path().join("sessions.db");
    let mut connection = Connection::open(&database_path).expect("temporary database should open");
    Migrations::from_slice(&MIGRATION_ENTRIES[..13])
        .to_latest(&mut connection)
        .expect("version thirteen schema should be created");
    connection
        .execute_batch(
            "CREATE TABLE session_replacement (
                 session_id TEXT PRIMARY KEY,
                 replaced_by_session_id TEXT NOT NULL
             );
             INSERT INTO session_replacement VALUES ('fixture-old', 'fixture-new');
             INSERT INTO watch_web_peer_out_message (
                 from_session_id, to_session_id, content, sent_at
             ) VALUES (
                 'fixture-sender', 'fixture-target', 'Fixture message', '2026-01-01T00:00:00Z'
             );",
        )
        .expect("legacy rewind schema should be created");
    connection
        .pragma_update(None, "user_version", 14)
        .expect("legacy schema version should be stored");
    drop(connection);

    let connection = super::super::open_connection(temp_dir.path())
        .expect("legacy version fourteen database should migrate to the latest schema");
    let actual = connection
        .query_row(
            indoc! {"
                SELECT
                    (SELECT replaced_by_session_id FROM session_replacement WHERE session_id = 'fixture-old'),
                    EXISTS(SELECT 1 FROM pragma_table_info('watch_web_peer_out_message') WHERE name = 'failed'),
                    (SELECT failed FROM watch_web_peer_out_message WHERE from_session_id = 'fixture-sender'),
                    EXISTS(SELECT 1 FROM pragma_table_info('agent_todo') WHERE name = 'resolution')
            "},
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            },
        )
        .expect("legacy replacement and added columns should be queryable");
    let actual_user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("database migration version should be readable");

    assert_eq!(
        (actual, actual_user_version),
        (
            ("fixture-new".to_string(), true, 0, true),
            current_version()
        ),
    );
}

#[test]
fn adds_todo_resolution_to_master_version_fourteen_database() {
    let temp_dir = tempfile::tempdir().expect("temporary directory should be created");
    let database_path = temp_dir.path().join("sessions.db");
    let mut connection = Connection::open(&database_path).expect("temporary database should open");
    Migrations::from_slice(&MIGRATION_ENTRIES[..14])
        .to_latest(&mut connection)
        .expect("master migration fourteen schema should be created");
    connection
        .execute(
            indoc! {"
                INSERT INTO agent_todo (
                    session_id, seq, title, created_at, completed_at, comment
                ) VALUES (
                    'sample-session', 1, 'Task', '2026-01-01T00:00:00Z',
                    '2026-01-02T00:00:00Z', 'Already handled'
                )
            "},
            [],
        )
        .expect("completed todo should be inserted before migration fifteen");
    drop(connection);

    let todos = super::super::agent_todos(temp_dir.path(), "sample-session")
        .expect("master version fourteen database should migrate and load todos");

    assert_eq!(
        todos,
        vec![super::super::AgentTodo {
            session_id: "sample-session".to_string(),
            seq: 1,
            title: "Task".to_string(),
            detail: None,
            created_at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .to_utc(),
            completed_at: Some(
                chrono::DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
                    .unwrap()
                    .to_utc(),
            ),
            resolution: Some(super::super::TodoResolution::Completed),
            comment: Some("Already handled".to_string()),
        }],
    );
}
