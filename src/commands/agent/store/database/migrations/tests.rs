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

#[derive(Clone, Copy)]
enum LegacyFeature {
    Decision,
    Proposal,
    PeerMessage,
}

#[rstest]
#[case::decision_only(
    LegacyFeature::Decision,
    Some("sample choice:sample reason"),
    None,
    None
)]
#[case::proposal_only(LegacyFeature::Proposal, None, Some("sample proposal"), None)]
#[case::peer_message_only(LegacyFeature::PeerMessage, None, None, Some("sample body"))]
fn upgrades_version_six_feature_layout(
    #[case] legacy_feature: LegacyFeature,
    #[case] expected_decision: Option<&str>,
    #[case] expected_proposal: Option<&str>,
    #[case] expected_peer_message: Option<&str>,
) {
    let temp_dir = tempfile::tempdir().expect("temporary directory should be created");
    let database_path = temp_dir.path().join("sessions.db");
    let mut connection = Connection::open(&database_path).expect("temporary database should open");
    Migrations::from_slice(&MIGRATION_ENTRIES[..5])
        .to_latest(&mut connection)
        .expect("base migrations should create the version five schema");

    match legacy_feature {
        LegacyFeature::Decision => connection
            .execute_batch(indoc! {
                "
                CREATE TABLE agent_decision (
                    session_id TEXT NOT NULL,
                    seq INTEGER NOT NULL,
                    title TEXT NOT NULL,
                    why TEXT NOT NULL,
                    considered TEXT NOT NULL,
                    easy_to_change TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    review TEXT,
                    reviewed_at TEXT,
                    PRIMARY KEY (session_id, seq)
                );
                INSERT INTO agent_decision (
                    session_id, seq, title, why, considered, easy_to_change, created_at
                ) VALUES (
                    'sample-session', 1, 'sample choice', 'sample reason',
                    'sample alternative', 'sample change', '2026-01-01T00:00:00Z'
                );
                "
            })
            .expect("legacy decision table should be created without extra schema objects"),
        LegacyFeature::Proposal => connection
            .execute_batch(indoc! {
                "
                CREATE TABLE agent_proposal (
                    session_id TEXT NOT NULL,
                    seq INTEGER NOT NULL,
                    body TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    answer TEXT,
                    resolved_at TEXT,
                    PRIMARY KEY (session_id, seq)
                );
                INSERT INTO agent_proposal (session_id, seq, body, created_at)
                VALUES ('sample-session', 1, 'sample proposal', '2026-01-01T00:00:00Z');
                "
            })
            .expect("legacy proposal table should be created without extra schema objects"),
        LegacyFeature::PeerMessage => connection
            .execute_batch(indoc! {
                "
                CREATE TABLE watch_web_peer_out_message (
                    id INTEGER PRIMARY KEY,
                    from_session_id TEXT NOT NULL,
                    to_session_id TEXT NOT NULL,
                    content TEXT NOT NULL,
                    sent_at TEXT NOT NULL
                );
                INSERT INTO watch_web_peer_out_message (
                    from_session_id, to_session_id, content, sent_at
                ) VALUES (
                    'sample-sender', 'sample-recipient', 'sample body', '2026-01-01T00:00:00Z'
                );
                "
            })
            .expect("legacy peer message table should be created without extra schema objects"),
    }

    connection
        .pragma_update(None, "user_version", 6)
        .expect("legacy migration version should be stored");
    drop(connection);

    let connection = super::super::open_connection(temp_dir.path())
        .expect("version six database should migrate to the combined schema");
    let actual = connection
        .query_row(
            "SELECT
                EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'agent_decision'),
                EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'agent_proposal'),
                EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'watch_web_peer_out_message'),
                EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'agent_proposal_unresolved_session_id'),
                EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = 'agent_proposal_delete_session'),
                EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'watch_web_peer_out_message_sender_target_time'),
                (SELECT title || ':' || why FROM agent_decision LIMIT 1),
                (SELECT body FROM agent_proposal LIMIT 1),
                (SELECT content FROM watch_web_peer_out_message LIMIT 1)",
            [],
            |row| {
                Ok((
                    (
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ),
                    (row.get(6)?, row.get(7)?, row.get(8)?),
                ))
            },
        )
        .expect("combined schema and retained rows should be queryable");
    let actual_user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("database migration version should be readable");

    assert_eq!(
        (actual, actual_user_version),
        (
            (
                (true, true, true, true, true, true),
                (
                    expected_decision.map(str::to_owned),
                    expected_proposal.map(str::to_owned),
                    expected_peer_message.map(str::to_owned),
                ),
            ),
            current_version(),
        ),
    );
}
