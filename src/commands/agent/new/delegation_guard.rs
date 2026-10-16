use anyhow::{Result, bail};

use crate::commands::agent::store;
use crate::shared::env_var::EnvVars;

pub(super) fn ensure_caller_not_delegated() -> Result<()> {
    let caller_session_id = EnvVars::load().own_session_id();
    ensure_session_not_delegated(caller_session_id.as_deref(), |session_id| {
        Ok(store::load_session(session_id)?.map(|session| session.is_delegated))
    })
}

fn ensure_session_not_delegated(
    caller_session_id: Option<&str>,
    load_is_delegated: impl FnOnce(&str) -> Result<Option<bool>>,
) -> Result<()> {
    let Some(caller_session_id) = caller_session_id else {
        return Ok(());
    };

    if load_is_delegated(caller_session_id)?.unwrap_or(false) {
        bail!("A delegated session cannot start another agent session.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::cell::Cell;

    #[rstest]
    #[case::unknown_caller(None, None, None, Ok(()))]
    #[case::missing_record(Some("session-a"), None, Some("session-a"), Ok(()))]
    #[case::non_delegated_record(Some("session-a"), Some(false), Some("session-a"), Ok(()))]
    #[case::delegated_record(
        Some("session-a"),
        Some(true),
        Some("session-a"),
        Err("A delegated session cannot start another agent session.")
    )]
    fn blocks_only_known_delegated_callers(
        #[case] caller_session_id: Option<&str>,
        #[case] record_is_delegated: Option<bool>,
        #[case] expected_loaded_session_id: Option<&str>,
        #[case] expected_result: std::result::Result<(), &'static str>,
    ) {
        let loaded_session_id = Cell::new(None);
        let result = ensure_session_not_delegated(caller_session_id, |session_id| {
            loaded_session_id.set(Some(session_id.to_string()));
            Ok(record_is_delegated)
        })
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, loaded_session_id.into_inner()),
            (
                expected_result.map_err(str::to_owned),
                expected_loaded_session_id.map(str::to_owned),
            ),
        );
    }

    #[test]
    fn propagates_caller_session_lookup_errors() {
        let result = ensure_session_not_delegated(Some("session-a"), |_| {
            Err(anyhow::anyhow!("session lookup failed"))
        })
        .map_err(|error| error.to_string());

        assert_eq!(result, Err("session lookup failed".to_string()));
    }
}
