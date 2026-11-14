use std::collections::HashMap;

use crate::commands::agent::types::Session;

use super::super::messages::RouteLoaders;

pub(super) fn question_previews(
    sessions: &[Session],
    loaders: &RouteLoaders,
) -> anyhow::Result<HashMap<String, String>> {
    let mut previews = HashMap::new();
    for session in sessions
        .iter()
        .filter(|session| session.is_current_tool_named("AskUserQuestion"))
    {
        match loaders.question_preview(session) {
            Ok(Some(preview)) => {
                previews.insert(session.session_id.clone(), preview);
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(
                session = %session.session_id,
                %error,
                "failed to load session question preview",
            ),
        }
    }
    Ok(previews)
}
