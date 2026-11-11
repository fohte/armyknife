use std::collections::{HashMap, hash_map::Entry};

use anyhow::Result;

use crate::commands::agent::claude_sessions;
use crate::commands::agent::types::Session;

use super::{ChatItem, ParentSession};

pub(super) fn parent_session(
    session: &Session,
    load_session: &mut impl FnMut(&str) -> Result<Option<Session>>,
) -> Result<Option<ParentSession>> {
    let Some(parent_id) = session.ancestor_session_ids.last() else {
        return Ok(None);
    };
    let Some(parent) = load_session(parent_id)? else {
        return Ok(None);
    };
    let title = resolved_session_title(&parent)
        .or_else(|| {
            parent
                .cwd
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "parent session".to_string());
    Ok(Some(ParentSession {
        session_id: parent.session_id,
        title,
    }))
}

pub(super) fn resolve_peer_session_titles(
    messages: &mut [ChatItem],
    load_session: &mut impl FnMut(&str) -> Result<Option<Session>>,
) -> Result<()> {
    let mut titles_by_id = HashMap::new();
    for message in messages {
        let ChatItem::Peer {
            session_id,
            session_title,
            ..
        } = message
        else {
            continue;
        };
        if session_title.is_some() || !is_resolvable_session_id(session_id) {
            continue;
        }
        if let Entry::Vacant(entry) = titles_by_id.entry(session_id.clone()) {
            let title =
                load_session(session_id)?.and_then(|session| resolved_session_title(&session));
            entry.insert(title);
        }
        *session_title = titles_by_id.get(session_id).cloned().flatten();
    }
    Ok(())
}

fn is_resolvable_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id != "unknown"
        && !session_id.contains('/')
        && !session_id.contains('\\')
        && !session_id.contains("..")
}

fn resolved_session_title(session: &Session) -> Option<String> {
    session
        .label
        .clone()
        .or_else(|| claude_sessions::get_session_title(&session.cwd, &session.session_id))
}
