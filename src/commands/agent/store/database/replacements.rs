use std::collections::HashSet;
use std::path::Path;

use anyhow::{Result, ensure};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{
    decode_session, increment_revision, open_connection, validate_session_id, write_session,
};
use crate::commands::agent::types::{Session, SessionStatus};

pub(crate) fn save_rewind_sessions(
    sessions_dir: &Path,
    previous: &Session,
    forked: &Session,
) -> Result<()> {
    ensure!(
        previous.session_id != forked.session_id,
        "rewound session ID must differ from the original"
    );
    ensure!(
        previous.status == SessionStatus::Ended,
        "rewound session must be marked ended"
    );
    ensure!(
        previous.ancestor_session_ids == forked.ancestor_session_ids,
        "rewound session must preserve the original ancestors"
    );
    let mut connection = open_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let previous_revision = write_session(&transaction, previous)?;
    write_session(&transaction, forked)?;
    transaction.execute(
        "INSERT INTO session_replacement (session_id, replaced_by_session_id)
         VALUES (?1, ?2)
         ON CONFLICT(session_id) DO UPDATE SET
             replaced_by_session_id = excluded.replaced_by_session_id",
        params![previous.session_id, forked.session_id],
    )?;

    let descendants = {
        let mut statement = transaction.prepare("SELECT session_id, data FROM sessions")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (session_id, data) in descendants {
        if session_id == previous.session_id || session_id == forked.session_id {
            continue;
        }
        let Some(mut descendant) = decode_session(&session_id, &data) else {
            continue;
        };
        let mut changed = false;
        for ancestor_id in &mut descendant.ancestor_session_ids {
            if ancestor_id == &previous.session_id {
                *ancestor_id = forked.session_id.clone();
                changed = true;
            }
        }
        if changed {
            write_session(&transaction, &descendant)?;
        }
    }

    transaction.execute(
        "INSERT INTO watch_web_session_state (session_id, is_today, primary_task_id, watch_web_user_marker)
         SELECT ?2, is_today, primary_task_id, watch_web_user_marker
         FROM watch_web_session_state WHERE session_id = ?1
         ON CONFLICT(session_id) DO UPDATE SET
             is_today = excluded.is_today,
             primary_task_id = excluded.primary_task_id,
             watch_web_user_marker = excluded.watch_web_user_marker",
        params![previous.session_id, forked.session_id],
    )?;
    let forked_revision = increment_revision(&transaction)?;
    transaction.execute(
        "UPDATE sessions SET rev = ?1 WHERE session_id = ?2",
        params![forked_revision, forked.session_id],
    )?;
    ensure!(
        previous_revision < forked_revision,
        "rewind session revisions are not increasing"
    );
    transaction.commit()?;
    Ok(())
}

pub(crate) fn all_unreplaced_sessions(sessions_dir: &Path) -> Result<Vec<Session>> {
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }
    let connection = open_connection(sessions_dir)?;
    let mut statement = connection.prepare(
        "SELECT sessions.session_id, sessions.data
         FROM sessions
         WHERE NOT EXISTS (
             SELECT 1 FROM session_replacement
             WHERE session_replacement.session_id = sessions.session_id
         )",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut sessions = Vec::new();
    for row in rows {
        let (session_id, data) = row?;
        if let Some(session) = decode_session(&session_id, &data) {
            sessions.push(session);
        }
    }
    Ok(sessions)
}

pub(crate) fn resolve_current_session_id(sessions_dir: &Path, session_id: &str) -> Result<String> {
    validate_session_id(session_id)?;
    let connection = open_connection(sessions_dir)?;
    let mut current = session_id.to_string();
    let mut visited = HashSet::new();
    loop {
        ensure!(
            visited.insert(current.clone()),
            "session replacement cycle includes {current}"
        );
        let replacement = connection
            .query_row(
                "SELECT replaced_by_session_id FROM session_replacement WHERE session_id = ?1",
                [&current],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(replacement) = replacement else {
            return Ok(current);
        };
        validate_session_id(&replacement)?;
        current = replacement;
    }
}
