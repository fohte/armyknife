use indoc::indoc;
use rstest::{fixture, rstest};
use rusqlite::Connection;
use rusqlite_migration::{Error as MigrationError, MigrationDefinitionError, Migrations};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use super::*;

#[derive(Clone, Copy, Debug)]
enum LegacySchema {
    FileAttachmentsAtVersionFifteen,
    FileAttachmentsAtVersionSixteen,
    QueuedPromptColumnsAtVersionSixteen,
}

struct DatabaseFixture {
    temp_dir: tempfile::TempDir,
    connection: Connection,
}

#[fixture]
fn database_fixture() -> DatabaseFixture {
    let temp_dir = tempfile::tempdir().expect("temporary directory should be created");
    let connection = Connection::open(temp_dir.path().join("sessions.db"))
        .expect("temporary database should open");
    DatabaseFixture {
        temp_dir,
        connection,
    }
}

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

#[rstest]
fn preserves_future_schema_without_current_tables() {
    let mut connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .execute_batch("CREATE TABLE future_state (value TEXT NOT NULL);")
        .expect("future schema should be created");
    connection
        .execute("INSERT INTO future_state (value) VALUES ('preserved')", [])
        .expect("future schema data should be inserted");
    connection
        .pragma_update(None, "user_version", current_version() + 1)
        .expect("future migration version should be stored");

    migrate_if_needed(&mut connection).expect("newer database versions should be left untouched");

    let table_names = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .expect("future table query should prepare")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("future table names should be queried")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("future table names should be collected");
    let column_names = connection
        .prepare("SELECT name FROM pragma_table_info('future_state') ORDER BY cid")
        .expect("future column query should prepare")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("future column names should be queried")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("future column names should be collected");
    let value: String = connection
        .query_row("SELECT value FROM future_state", [], |row| row.get(0))
        .expect("future schema data should remain queryable");
    let user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("future migration version should be readable");

    assert_eq!(
        (table_names, column_names, value, user_version),
        (
            vec!["future_state".to_string()],
            vec!["value".to_string()],
            "preserved".to_string(),
            current_version() + 1,
        ),
    );
}

#[rstest]
#[case::queued_prompt_columns(
    "CREATE TABLE watch_web_queued_prompt (
        id INTEGER PRIMARY KEY,
        session_id TEXT NOT NULL,
        content TEXT NOT NULL,
        sent_at TEXT NOT NULL
    );",
    "watch_web_queued_prompt",
    vec!["id", "session_id", "content", "sent_at", "attachments", "message_id"],
    ensure_queued_prompt_columns
)]
#[case::todo_resolution_column(
    "CREATE TABLE agent_todo (
        session_id TEXT NOT NULL,
        seq INTEGER NOT NULL,
        title TEXT NOT NULL,
        created_at TEXT NOT NULL,
        PRIMARY KEY (session_id, seq)
    );",
    "agent_todo",
    vec!["session_id", "seq", "title", "created_at", "resolution"],
    ensure_todo_resolution_column
)]
fn concurrent_column_repairs_are_idempotent(
    #[case] schema: &str,
    #[case] table: &str,
    #[case] expected_columns: Vec<&str>,
    #[case] repair: fn(&mut Connection) -> anyhow::Result<()>,
    database_fixture: DatabaseFixture,
) {
    let DatabaseFixture {
        temp_dir,
        connection,
    } = database_fixture;
    connection
        .execute_batch(schema)
        .expect("legacy schema should be created");
    drop(connection);

    let barrier = Arc::new(Barrier::new(2));
    let workers = (0..2)
        .map(|_| {
            let database_path = temp_dir.path().join("sessions.db");
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let mut connection = Connection::open(database_path)
                    .expect("worker database connection should open");
                connection
                    .busy_timeout(Duration::from_secs(5))
                    .expect("worker should wait for concurrent schema changes");
                barrier.wait();
                repair(&mut connection).expect("concurrent schema repairs should succeed");
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().expect("migration worker should not panic");
    }

    let connection = Connection::open(temp_dir.path().join("sessions.db"))
        .expect("migrated database should open");
    let actual = connection
        .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
        .expect("repaired schema query should prepare")
        .query_map([table], |row| row.get::<_, String>(0))
        .expect("repaired schema columns should be queried")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("repaired schema columns should be collected");

    assert_eq!(
        actual,
        expected_columns
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
    );
}

#[rstest]
fn adds_permission_request_mailbox_to_version_nine_database(database_fixture: DatabaseFixture) {
    let DatabaseFixture {
        temp_dir,
        mut connection,
    } = database_fixture;
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
#[rstest]
fn resolves_legacy_queued_prompts_without_message_ids(database_fixture: DatabaseFixture) {
    let DatabaseFixture {
        temp_dir,
        mut connection,
    } = database_fixture;
    Migrations::from_slice(&MIGRATION_ENTRIES[..16])
        .to_latest(&mut connection)
        .expect("version sixteen migrations should create the message id schema");
    connection
        .execute(
            indoc! {"
                INSERT INTO watch_web_queued_prompt (
                session_id, content, sent_at, message_id
                ) VALUES
                ('fixture-session', 'Legacy prompt.', '2026-01-01T00:00:00Z', NULL),
                ('fixture-session', 'New prompt.', '2026-01-01T00:00:01Z', 'fixture-message-cyan')",
            },
            [],
        )
        .expect("legacy and id-bearing queued prompts should be inserted");
    drop(connection);

    let connection = super::super::open_connection(temp_dir.path())
        .expect("version sixteen database should migrate to the current schema");
    connection
        .execute(
            indoc! {"
                INSERT INTO watch_web_queued_prompt (session_id, content, sent_at)
                VALUES ('fixture-session', 'Old writer prompt.', '2026-01-01T00:00:02Z')
            "},
            [],
        )
        .expect("old writer should be able to insert into the migrated schema");
    let actual = connection
        .prepare(indoc! {"
                SELECT message_id, resolved, attachments
                FROM watch_web_queued_prompt ORDER BY id
            "})
        .expect("queued prompt migration query should prepare")
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("queued prompts should be queryable")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("queued prompts should deserialize");

    assert_eq!(
        actual,
        vec![
            (None, 1, "[]".to_string()),
            (
                Some("fixture-message-cyan".to_string()),
                0,
                "[]".to_string()
            ),
            (None, 1, "[]".to_string()),
        ],
    );
}

#[rstest]
fn adds_todo_resolution_to_master_version_fourteen_database(database_fixture: DatabaseFixture) {
    let DatabaseFixture {
        temp_dir,
        mut connection,
    } = database_fixture;
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
    let actual = todos
        .first()
        .map(|todo| (todo.title.clone(), todo.resolution, todo.comment.clone()));

    assert_eq!(
        actual,
        Some((
            "Task".to_string(),
            Some(super::super::TodoResolution::Completed),
            Some("Already handled".to_string()),
        )),
    );
}

#[fixture]
fn version_fourteen_database() -> Connection {
    let mut connection = Connection::open_in_memory().expect("in-memory database should open");
    Migrations::from_slice(&MIGRATION_ENTRIES[..14])
        .to_latest(&mut connection)
        .expect("version fourteen schema should be created");
    connection
        .execute(
            "INSERT INTO agent_todo (session_id, seq, title, created_at)
             VALUES ('sample-session', 1, 'Task', '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("existing todo should be inserted");
    connection
        .execute(
            "INSERT INTO watch_web_queued_prompt (id, session_id, content, sent_at)
             VALUES (1, 'sample-session', 'Queued content', '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("existing queued prompt should be inserted");
    connection
}

fn insert_legacy_file(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO agent_file (
                session_id, id, position, name, size, created_at
            ) VALUES (
                'sample-session', 'fixture-file-id', 0, 'fixture.pdf', 7,
                '2026-01-01T00:00:00Z'
            )",
            [],
        )
        .expect("existing attachment should be inserted");
}

fn set_legacy_schema(connection: &Connection, history: LegacySchema) {
    match history {
        LegacySchema::FileAttachmentsAtVersionFifteen => {
            connection
                .execute_batch(AGENT_FILE_SCHEMA)
                .expect("version fifteen file table should be created");
            insert_legacy_file(connection);
            connection
                .pragma_update(None, "user_version", 15)
                .expect("version fifteen should be recorded");
        }
        LegacySchema::FileAttachmentsAtVersionSixteen => {
            connection
                .execute_batch("ALTER TABLE agent_todo ADD COLUMN resolution TEXT;")
                .expect("todo resolution column should be created");
            connection
                .execute_batch(AGENT_FILE_SCHEMA)
                .expect("version sixteen file table should be created");
            insert_legacy_file(connection);
            connection
                .pragma_update(None, "user_version", 16)
                .expect("version sixteen should be recorded");
        }
        LegacySchema::QueuedPromptColumnsAtVersionSixteen => {
            connection
                .execute_batch(indoc! {
                    "
                    ALTER TABLE agent_todo ADD COLUMN resolution TEXT;
                    ALTER TABLE watch_web_queued_prompt
                        ADD COLUMN attachments TEXT NOT NULL DEFAULT '[]';
                    ALTER TABLE watch_web_queued_prompt ADD COLUMN message_id TEXT;
                    "
                })
                .expect("version sixteen queued prompt columns should be created");
            connection
                .pragma_update(None, "user_version", 16)
                .expect("version sixteen should be recorded");
        }
    }
}

#[rstest]
#[case::file_attachments_at_version_fifteen(LegacySchema::FileAttachmentsAtVersionFifteen, 1)]
#[case::file_attachments_at_version_sixteen(LegacySchema::FileAttachmentsAtVersionSixteen, 1)]
#[case::queued_prompt_columns_at_version_sixteen(
    LegacySchema::QueuedPromptColumnsAtVersionSixteen,
    0
)]
fn repairs_conflicting_migration_histories(
    #[case] history: LegacySchema,
    #[case] expected_file_count: i64,
    mut version_fourteen_database: Connection,
) {
    set_legacy_schema(&version_fourteen_database, history);
    migrate_if_needed(&mut version_fourteen_database)
        .expect("legacy schema should migrate to the current version");

    let actual = version_fourteen_database
        .query_row(
            indoc! {
                "
                SELECT
                    EXISTS(SELECT 1 FROM pragma_table_info('agent_todo') WHERE name = 'resolution'),
                    EXISTS(SELECT 1 FROM pragma_table_info('watch_web_queued_prompt') WHERE name = 'attachments'),
                    EXISTS(SELECT 1 FROM pragma_table_info('watch_web_queued_prompt') WHERE name = 'message_id'),
                    EXISTS(SELECT 1 FROM pragma_table_info('watch_web_queued_prompt') WHERE name = 'resolved'),
                    EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'agent_file_session_id_created_at'),
                    EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = 'agent_file_delete_session'),
                    (SELECT title FROM agent_todo WHERE session_id = 'sample-session'),
                    (SELECT resolution FROM agent_todo WHERE session_id = 'sample-session'),
                    (SELECT attachments FROM watch_web_queued_prompt WHERE id = 1),
                    (SELECT message_id FROM watch_web_queued_prompt WHERE id = 1),
                    (SELECT resolved FROM watch_web_queued_prompt WHERE id = 1),
                    (SELECT COUNT(*) FROM agent_file WHERE id = 'fixture-file-id')
                "
            },
            [],
            |row| {
                Ok((
                    row.get::<_, bool>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, bool>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, i64>(11)?,
                ))
            },
        )
        .expect("repaired schemas and existing records should remain queryable");
    let user_version: i64 = version_fourteen_database
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("database migration version should be readable");

    assert_eq!(
        (actual, user_version),
        (
            (
                true,
                true,
                true,
                true,
                true,
                true,
                "Task".to_string(),
                None,
                "[]".to_string(),
                None,
                1,
                expected_file_count,
            ),
            current_version(),
        ),
    );
}
