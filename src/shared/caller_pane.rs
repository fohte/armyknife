use crate::commands::agent::store;
use crate::commands::agent::types::Session;
use crate::infra::tmux;
use crate::shared::env_var::EnvVars;

/// Resolves the caller's pane from `TMUX_PANE` or the current agent session.
///
/// A supplied session preserves callers that already loaded it under a lock.
/// When no session is supplied, `TMUX_PANE` keeps precedence so existing tmux
/// callers continue to target their actual pane.
pub(crate) fn resolve_caller_pane_id(session: Option<&Session>) -> Option<String> {
    if let Some(session) = session {
        return pane_id_for_session(session);
    }

    resolve_pane_id(tmux::current_pane_id_from_env(), || {
        // Preserve the configured-terminal fallback if the session store is unavailable.
        let session = EnvVars::load()
            .own_session_id()
            .and_then(|session_id| store::load_session(&session_id).ok().flatten());
        session.as_ref().and_then(pane_id_for_session)
    })
}

fn pane_id_for_session(session: &Session) -> Option<String> {
    session
        .tmux_info
        .as_ref()
        .map(|tmux_info| tmux_info.pane_id.clone())
}

fn resolve_pane_id(
    env_pane_id: Option<String>,
    session_pane_id: impl FnOnce() -> Option<String>,
) -> Option<String> {
    env_pane_id.or_else(session_pane_id)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::resolve_pane_id;

    #[rstest]
    #[case::environment_pane_takes_precedence(
        Some("%environment"),
        Some("%session"),
        Some("%environment"),
        false
    )]
    #[case::session_pane_is_used_without_environment(
        None,
        Some("%session"),
        Some("%session"),
        true
    )]
    #[case::no_pane_is_available(None, None, None, true)]
    fn resolves_session_or_environment_pane(
        #[case] env_pane_id: Option<&str>,
        #[case] session_pane_id: Option<&str>,
        #[case] expected: Option<&str>,
        #[case] session_lookup_expected: bool,
    ) {
        let mut session_lookup_called = false;
        let actual = resolve_pane_id(env_pane_id.map(str::to_string), || {
            session_lookup_called = true;
            session_pane_id.map(str::to_string)
        });
        assert_eq!(
            (actual, session_lookup_called),
            (expected.map(str::to_string), session_lookup_expected),
        );
    }
}
