use std::collections::HashMap;
use std::sync::Arc;

use crate::commands::agent::types::Session;
use crate::commands::agent::vibe::{self, VibeState};
use crate::shared::config::Config;

pub(super) type VibeAncestorLoader =
    Arc<dyn Fn(&str) -> anyhow::Result<Option<Session>> + Send + Sync>;

pub(super) fn states_for_sessions(
    sessions: &[Session],
    all_sessions: &[Session],
    repo_by_id: &HashMap<String, String>,
    config: &Config,
    mut load_missing_ancestor: impl FnMut(&str) -> anyhow::Result<Option<Session>>,
) -> anyhow::Result<Vec<VibeState>> {
    let all_sessions_by_id = all_sessions
        .iter()
        .map(|session| (session.session_id.as_str(), session))
        .collect::<HashMap<_, _>>();
    let mut missing_ancestors_by_id: HashMap<String, Option<Session>> = HashMap::new();

    sessions
        .iter()
        .map(|session| {
            let repo_identity = repo_by_id
                .get(&session.session_id)
                .and_then(|repo| repo.split_once('/'));
            vibe::state_for_with_ancestor_loader(session, repo_identity, config, |ancestor_id| {
                if let Some(ancestor) = all_sessions_by_id.get(ancestor_id) {
                    return Ok(Some((*ancestor).clone()));
                }
                if let Some(ancestor) = missing_ancestors_by_id.get(ancestor_id) {
                    return Ok(ancestor.clone());
                }

                let ancestor = load_missing_ancestor(ancestor_id)?;
                missing_ancestors_by_id.insert(ancestor_id.to_owned(), ancestor.clone());
                Ok(ancestor)
            })
        })
        .collect()
}
