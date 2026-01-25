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

    focus_tmux_pane(&tmux_info)?;

    Ok(())
}

/// Extracts TmuxInfo from an optional Session, returning appropriate errors.
fn extract_tmux_info(session_id: &str, session: Option<Session>) -> Result<TmuxInfo, CcError> {
    let session = session.ok_or_else(|| CcError::SessionNotFound(session_id.to_string()))?;

    session
        .tmux_info
        .ok_or_else(|| CcError::NoTmuxInfo(session_id.to_string()))
}

/// Focuses the tmux pane specified by TmuxInfo.
/// Runs `tmux select-window` followed by `tmux select-pane`.
fn focus_tmux_pane(info: &TmuxInfo) -> Result<()> {
    let window_target = format!("{}:{}", info.session_name, info.window_index);
    tmux::select_window(&window_target)?;
    tmux::select_pane(&info.pane_id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::SessionStatus;
    use chrono::Utc;
    use std::path::PathBuf;

    fn create_test_session(id: &str, tmux_info: Option<TmuxInfo>) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: Some("/dev/ttys001".to_string()),
            tmux_info,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
        }
    }

    #[test]
    fn test_extract_tmux_info_session_not_found() {
        let result = extract_tmux_info("nonexistent", None);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, CcError::SessionNotFound(_)));
        assert!(err.to_string().contains("Session not found"));
    }

    #[test]
    fn test_extract_tmux_info_no_tmux_info() {
        let session = create_test_session("no-tmux", None);

        let result = extract_tmux_info("no-tmux", Some(session));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, CcError::NoTmuxInfo(_)));
        assert!(err.to_string().contains("no tmux information"));
    }

    #[test]
    fn test_extract_tmux_info_success() {
        let tmux_info = TmuxInfo {
            session_name: "main".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        };
        let session = create_test_session("with-tmux", Some(tmux_info.clone()));

        let result = extract_tmux_info("with-tmux", Some(session));

        assert!(result.is_ok());
        let extracted = result.unwrap();
        assert_eq!(extracted.session_name, "main");
        assert_eq!(extracted.window_name, "editor");
        assert_eq!(extracted.window_index, 0);
        assert_eq!(extracted.pane_id, "%0");
    }
}
