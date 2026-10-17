use anyhow::{Result, bail};

use super::super::error::CcError;
use super::super::types::Session;
use super::CloseArgs;
use crate::shared::env_var::EnvVars;

pub(super) fn parent_session_id(args: &CloseArgs) -> Result<Option<String>> {
    if !args.child {
        return Ok(None);
    }
    require_parent_session_id(|| EnvVars::load().own_session_id()).map(Some)
}

pub(super) fn require_parent_session_id(
    self_session_id: impl FnOnce() -> Option<String>,
) -> Result<String> {
    self_session_id().ok_or_else(|| CcError::SelfSessionUnknown.into())
}

pub(super) fn ensure_target_session(
    session: Option<&Session>,
    parent_session_id: Option<&str>,
) -> Result<()> {
    let Some(parent_session_id) = parent_session_id else {
        return Ok(());
    };
    let Some(session) = session else {
        bail!("The target worktree has no tracked agent session; cannot verify --child");
    };
    if !session.is_direct_child_of(parent_session_id) {
        bail!(
            "Agent session `{}` is not a direct child of the calling session",
            session.session_id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use chrono::Utc;
    use rstest::rstest;

    use super::*;
    use crate::commands::agent::types::{Engine, SessionStatus};

    fn session(ancestor_session_ids: Vec<String>) -> Session {
        Session {
            archived_at: None,
            session_id: "target-session".to_string(),
            pr_context: None,
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/target-worktree"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            launched_with_vibe: false,
            work_type_pinned: false,
            is_delegated: false,
            ancestor_session_ids,
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
        }
    }

    #[rstest]
    #[case::unknown_self_session_id(None, Err(CcError::SelfSessionUnknown.to_string()))]
    #[case::returns_self_session_id(Some("parent"), Ok("parent".to_string()))]
    fn resolves_self_session_id(
        #[case] self_session_id: Option<&str>,
        #[case] expected: std::result::Result<String, String>,
    ) {
        let actual = require_parent_session_id(|| self_session_id.map(str::to_string))
            .map_err(|error| error.to_string());

        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case::direct_child(vec!["root", "parent"], Some("parent"), true)]
    #[case::grandchild(vec!["root", "parent", "child"], Some("parent"), false)]
    #[case::unrelated(vec!["root", "other"], Some("parent"), false)]
    #[case::missing_parent(Vec::new(), Some("parent"), false)]
    fn validates_only_direct_child_sessions(
        #[case] ancestors: Vec<&str>,
        #[case] session_parent_id: Option<&str>,
        #[case] expected_success: bool,
    ) {
        let session = session(ancestors.into_iter().map(str::to_string).collect());
        let result = ensure_target_session(Some(&session), session_parent_id);

        assert_eq!(result.is_ok(), expected_success);
    }

    #[test]
    fn refuses_a_worktree_without_a_tracked_session() {
        let result = ensure_target_session(None, Some("parent"));

        assert_eq!(
            result.map_err(|error| error.to_string()),
            Err(
                "The target worktree has no tracked agent session; cannot verify --child"
                    .to_string()
            ),
        );
    }
}
