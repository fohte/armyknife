use anyhow::{Result, bail};

use crate::commands::agent::store;
use crate::commands::agent::types::ChildLaunchPermission;
use crate::shared::env_var::EnvVars;

pub(super) fn ensure_caller_may_launch_children() -> Result<()> {
    let caller_session_id = EnvVars::load().own_session_id();
    ensure_session_may_launch_children(caller_session_id.as_deref(), |session_id| {
        Ok(store::load_session(session_id)?.map(|session| session.child_launch))
    })
}

fn ensure_session_may_launch_children(
    caller_session_id: Option<&str>,
    load_permission: impl FnOnce(&str) -> Result<Option<ChildLaunchPermission>>,
) -> Result<()> {
    let Some(caller_session_id) = caller_session_id else {
        return Ok(());
    };

    if load_permission(caller_session_id)?.unwrap_or_default() == ChildLaunchPermission::Deny {
        bail!("The current session is not allowed to start child sessions.");
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
    #[case::allowed_record(
        Some("session-a"),
        Some(ChildLaunchPermission::Allow),
        Some("session-a"),
        Ok(())
    )]
    #[case::denied_record(
        Some("session-a"),
        Some(ChildLaunchPermission::Deny),
        Some("session-a"),
        Err("The current session is not allowed to start child sessions.")
    )]
    fn checks_only_the_calling_sessions_child_launch_permission(
        #[case] caller_session_id: Option<&str>,
        #[case] permission: Option<ChildLaunchPermission>,
        #[case] expected_loaded_session_id: Option<&str>,
        #[case] expected_result: std::result::Result<(), &'static str>,
    ) {
        let loaded_session_id = Cell::new(None);
        let result = ensure_session_may_launch_children(caller_session_id, |session_id| {
            loaded_session_id.set(Some(session_id.to_string()));
            Ok(permission)
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
        let result = ensure_session_may_launch_children(Some("session-a"), |_| {
            Err(anyhow::anyhow!("session lookup failed"))
        })
        .map_err(|error| error.to_string());

        assert_eq!(result, Err("session lookup failed".to_string()));
    }
}
