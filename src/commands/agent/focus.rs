use anyhow::Result;
use clap::Args;

use super::error::CcError;
use super::store;
#[cfg(test)]
use super::types::Engine;
use super::types::{Session, TmuxInfo};
use crate::infra::tmux;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct FocusArgs {
    /// Session ID to focus
    pub session_id: String,
    /// Review pane ID to focus before falling back to the session pane
    #[arg(long)]
    pub review_pane_id: Option<String>,
}

/// Runs the focus command.
/// Switches tmux focus to the pane associated with the specified session.
pub fn run(args: &FocusArgs) -> Result<()> {
    focus_pane_with_fallback(
        args.review_pane_id.as_deref(),
        |pane_id| tmux::focus_pane(pane_id).map_err(anyhow::Error::from),
        || {
            let session = store::load_session(&args.session_id)?;
            let tmux_info = extract_tmux_info(&args.session_id, session)?;
            tmux::focus_pane(&tmux_info.pane_id)?;
            Ok(())
        },
    )?;

    Ok(())
}

fn focus_pane_with_fallback(
    review_pane_id: Option<&str>,
    mut focus_review_pane: impl FnMut(&str) -> Result<()>,
    focus_session_pane: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if review_pane_id.is_some_and(|pane_id| focus_review_pane(pane_id).is_ok()) {
        return Ok(());
    }

    focus_session_pane()
}

/// Extracts TmuxInfo from an optional Session, returning appropriate errors.
fn extract_tmux_info(session_id: &str, session: Option<Session>) -> Result<TmuxInfo, CcError> {
    let session = session.ok_or_else(|| CcError::SessionNotFound(session_id.to_string()))?;

    session
        .tmux_info
        .ok_or_else(|| CcError::NoTmuxInfo(session_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::SessionStatus;
    use anyhow::anyhow;
    use chrono::Utc;
    use rstest::rstest;
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn create_test_session(tmux_info: Option<TmuxInfo>) -> Session {
        Session {
            session_id: "test-123".to_string(),
            work_type: None,
            work_type_pinned: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            pending_agent_task_ids: std::collections::BTreeSet::new(),
            pending_permission_agent_ids: std::collections::BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            engine: Engine::Claude,
        }
    }

    #[rstest]
    #[case::session_not_found(None, "Session not found")]
    #[case::no_tmux_info(Some(create_test_session(None)), "no tmux information")]
    fn test_extract_tmux_info_errors(
        #[case] session: Option<Session>,
        #[case] expected_error: &str,
    ) {
        let result = extract_tmux_info("test-id", session);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(expected_error));
    }

    #[test]
    fn test_extract_tmux_info_success() {
        let tmux_info = TmuxInfo {
            session_name: "main".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        };
        let session = create_test_session(Some(tmux_info));

        let result = extract_tmux_info("test-id", Some(session));

        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.session_name, "main");
        assert_eq!(info.window_name, "editor");
        assert_eq!(info.window_index, 0);
        assert_eq!(info.pane_id, "%0");
    }

    #[rstest]
    #[case::review_pane_is_open(
        Some("%review"),
        vec!["%review"],
        vec![],
        Ok(()),
    )]
    #[case::closed_review_pane_falls_back(
        Some("%review"),
        vec!["%review", "%agent"],
        vec!["%review"],
        Ok(()),
    )]
    #[case::without_review_pane_focuses_session(
        None,
        vec!["%agent"],
        vec![],
        Ok(()),
    )]
    #[case::fallback_error_is_returned(
        Some("%review"),
        vec!["%review", "%agent"],
        vec!["%review", "%agent"],
        Err("pane unavailable".to_string()),
    )]
    fn review_pane_focus_falls_back_to_session_pane(
        #[case] review_pane_id: Option<&str>,
        #[case] expected_pane_ids: Vec<&str>,
        #[case] unavailable_pane_ids: Vec<&str>,
        #[case] expected_result: std::result::Result<(), String>,
    ) {
        let focused_pane_ids = RefCell::new(Vec::new());
        let result = focus_pane_with_fallback(
            review_pane_id,
            |pane_id| {
                focused_pane_ids.borrow_mut().push(pane_id.to_string());
                if unavailable_pane_ids.contains(&pane_id) {
                    Err(anyhow!("pane unavailable"))
                } else {
                    Ok(())
                }
            },
            || {
                focused_pane_ids.borrow_mut().push("%agent".to_string());
                if unavailable_pane_ids.contains(&"%agent") {
                    Err(anyhow!("pane unavailable"))
                } else {
                    Ok(())
                }
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (focused_pane_ids.into_inner(), result),
            (
                expected_pane_ids.into_iter().map(str::to_string).collect(),
                expected_result
            )
        );
    }
}
