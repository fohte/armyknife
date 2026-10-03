use crate::commands::agent::store;
use crate::commands::agent::types::{Session, resolve_session_option};
use crate::infra::tmux;
use crate::shared::env_var::EnvVars;

/// Resolves the caller's pane from `TMUX_PANE` or the current agent session.
pub(crate) fn resolve_caller_pane_id() -> Option<String> {
    resolve_caller_pane_id_with(
        tmux::current_pane_id_from_env(),
        || EnvVars::load().own_session_id(),
        |session_id| store::load_session(session_id).ok().flatten(),
        |pane_id| resolve_session_option(|option| tmux::get_pane_option(pane_id, option)),
    )
}

/// Returns the pane recorded on a session that the caller has already loaded.
///
/// Callers holding a session-store lock use this to avoid acquiring it again.
pub(crate) fn pane_id_for_session(session: &Session) -> Option<String> {
    session
        .tmux_info
        .as_ref()
        .map(|tmux_info| tmux_info.pane_id.clone())
}

fn resolve_caller_pane_id_with(
    env_pane_id: Option<String>,
    own_session_id: impl FnOnce() -> Option<String>,
    load_session: impl FnOnce(&str) -> Option<Session>,
    pane_session_id: impl FnOnce(&str) -> Option<String>,
) -> Option<String> {
    if let Some(pane_id) = env_pane_id {
        return Some(pane_id);
    }

    let own_session_id = own_session_id()?;
    let session = load_session(&own_session_id)?;
    let pane_id = pane_id_for_session(&session)?;
    let pane_owner = pane_session_id(&pane_id)?;
    (pane_owner == own_session_id).then_some(pane_id)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use super::resolve_caller_pane_id_with;
    use crate::commands::agent::types::Session;

    struct Case {
        env_pane_id: Option<&'static str>,
        own_id: Option<&'static str>,
        stored_session_id: Option<&'static str>,
        stored_pane_id: Option<&'static str>,
        pane_owner: Option<&'static str>,
        expected_pane_id: Option<&'static str>,
        expected_loaded_session_ids: &'static [&'static str],
        expected_inspected_pane_ids: &'static [&'static str],
        expected_own_id_lookup: bool,
    }

    #[rstest]
    #[case::environment_pane_takes_precedence(Case {
        env_pane_id: Some("%environment"),
        own_id: Some("session-a"),
        stored_session_id: Some("session-a"),
        stored_pane_id: Some("%stored"),
        pane_owner: Some("session-a"),
        expected_pane_id: Some("%environment"),
        expected_loaded_session_ids: &[],
        expected_inspected_pane_ids: &[],
        expected_own_id_lookup: false,
    })]
    #[case::matching_stored_pane_is_used_without_environment(Case {
        env_pane_id: None,
        own_id: Some("session-a"),
        stored_session_id: Some("session-a"),
        stored_pane_id: Some("%stored"),
        pane_owner: Some("session-a"),
        expected_pane_id: Some("%stored"),
        expected_loaded_session_ids: &["session-a"],
        expected_inspected_pane_ids: &["%stored"],
        expected_own_id_lookup: true,
    })]
    #[case::stale_pane_for_another_session_is_rejected(Case {
        env_pane_id: None,
        own_id: Some("session-a"),
        stored_session_id: Some("session-a"),
        stored_pane_id: Some("%stored"),
        pane_owner: Some("session-b"),
        expected_pane_id: None,
        expected_loaded_session_ids: &["session-a"],
        expected_inspected_pane_ids: &["%stored"],
        expected_own_id_lookup: true,
    })]
    #[case::missing_own_session_id_has_no_fallback(Case {
        env_pane_id: None,
        own_id: None,
        stored_session_id: Some("session-a"),
        stored_pane_id: Some("%stored"),
        pane_owner: Some("session-a"),
        expected_pane_id: None,
        expected_loaded_session_ids: &[],
        expected_inspected_pane_ids: &[],
        expected_own_id_lookup: true,
    })]
    #[case::missing_stored_session_has_no_fallback(Case {
        env_pane_id: None,
        own_id: Some("session-a"),
        stored_session_id: None,
        stored_pane_id: Some("%stored"),
        pane_owner: Some("session-a"),
        expected_pane_id: None,
        expected_loaded_session_ids: &["session-a"],
        expected_inspected_pane_ids: &[],
        expected_own_id_lookup: true,
    })]
    #[case::missing_stored_pane_has_no_fallback(Case {
        env_pane_id: None,
        own_id: Some("session-a"),
        stored_session_id: Some("session-a"),
        stored_pane_id: None,
        pane_owner: Some("session-a"),
        expected_pane_id: None,
        expected_loaded_session_ids: &["session-a"],
        expected_inspected_pane_ids: &[],
        expected_own_id_lookup: true,
    })]
    fn resolves_caller_pane(#[case] case: Case) {
        let mut own_id_lookup_called = false;
        let mut loaded_session_ids = Vec::new();
        let mut inspected_pane_ids = Vec::new();
        let actual = resolve_caller_pane_id_with(
            case.env_pane_id.map(str::to_string),
            || {
                own_id_lookup_called = true;
                case.own_id.map(str::to_string)
            },
            |session_id| {
                loaded_session_ids.push(session_id.to_string());
                if case.stored_session_id == Some(session_id) {
                    case.stored_session_id
                        .map(|id| session(id, case.stored_pane_id))
                } else {
                    None
                }
            },
            |pane_id| {
                inspected_pane_ids.push(pane_id.to_string());
                case.pane_owner.map(str::to_string)
            },
        );

        assert_eq!(
            (
                actual,
                loaded_session_ids,
                inspected_pane_ids,
                own_id_lookup_called,
            ),
            (
                case.expected_pane_id.map(str::to_string),
                case.expected_loaded_session_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect(),
                case.expected_inspected_pane_ids
                    .iter()
                    .map(|pane_id| pane_id.to_string())
                    .collect(),
                case.expected_own_id_lookup,
            ),
        );
    }

    fn session(session_id: &str, pane_id: Option<&str>) -> Session {
        serde_json::from_value(json!({
            "session_id": session_id,
            "cwd": "/tmp/worktree",
            "transcript_path": null,
            "tmux_info": pane_id.map(|pane_id| json!({
                "session_name": "work",
                "window_name": "agent",
                "window_index": 0,
                "pane_id": pane_id,
            })),
            "status": "running",
            "created_at": "2024-01-02T03:04:05Z",
            "updated_at": "2024-01-02T03:04:05Z",
            "last_message": null,
        }))
        .expect("test session should deserialize")
    }
}
