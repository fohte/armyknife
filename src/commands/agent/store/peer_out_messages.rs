use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::database::{self, WatchWebPeerOutMessage};

pub(crate) fn insert_watch_web_peer_out_message_from(
    sessions_dir: &Path,
    from_session_id: &str,
    to_session_id: &str,
    content: &str,
    sent_at: DateTime<Utc>,
) -> Result<()> {
    database::insert_watch_web_peer_out_message(
        sessions_dir,
        from_session_id,
        to_session_id,
        content,
        sent_at,
    )
    .map(|_| ())
}

pub(crate) fn watch_web_peer_out_messages_from(
    sessions_dir: &Path,
    from_session_id: &str,
) -> Result<Vec<WatchWebPeerOutMessage>> {
    database::watch_web_peer_out_messages(sessions_dir, from_session_id)
}

pub(crate) fn insert_watch_web_peer_out_message(
    from_session_id: &str,
    to_session_id: &str,
    content: &str,
    sent_at: DateTime<Utc>,
) -> Result<()> {
    insert_watch_web_peer_out_message_from(
        &super::sessions_dir()?,
        from_session_id,
        to_session_id,
        content,
        sent_at,
    )
}
