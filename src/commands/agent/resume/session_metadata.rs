use std::path::Path;

use anyhow::Result;

use crate::commands::agent::store;
use crate::shared::env_var::parse_ancestor_session_ids;

pub(super) fn record_ancestor_session_ids_if_empty(
    sessions_dir: &Path,
    session_id: &str,
    ancestor_session_ids: &str,
) -> Result<()> {
    let ancestor_session_ids = parse_ancestor_session_ids(ancestor_session_ids);
    if ancestor_session_ids.is_empty() {
        return Ok(());
    }

    store::update_session_in(sessions_dir, session_id, |session| {
        if !session.ancestor_session_ids.is_empty() {
            return false;
        }

        session.ancestor_session_ids = ancestor_session_ids;
        true
    })
}
