use anyhow::Result;
use clap::Args;

use super::error::CcError;
use super::store;
use super::types::{Session, TmuxInfo};
use crate::infra::tmux;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct FocusArgs {
    /// Session ID to focus
    pub session_id: String,
}

/// Runs the focus command.
/// Switches tmux focus to the pane associated with the specified session.
pub fn run(args: &FocusArgs) -> Result<()> {
    let session = store::load_session(&args.session_id)?;
    let tmux_info = extract_tmux_info(&args.session_id, session)?;

    tmux::focus_pane(&tmux_info.pane_id)?;

    Ok(())
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
    use crate::commands::cc::types::SessionStatus;
    use chrono::Utc;
    use rstest::rstest;
    use std::path::PathBuf;

    fn create_test_session(tmux_info: Option<TmuxInfo>) -> Session {
        Session {
            session_id: "test-123".to_string(),
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
}
