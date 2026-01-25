use std::process::Command;

use anyhow::{Result, bail};
use clap::Args;

use super::error::CcError;
use super::store;
use super::types::TmuxInfo;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct FocusArgs {
    /// Session ID to focus
    pub session_id: String,
}

/// Runs the focus command.
/// Switches tmux focus to the pane associated with the specified session.
pub fn run(args: &FocusArgs) -> Result<()> {
    let session = store::load_session(&args.session_id)?
        .ok_or_else(|| CcError::SessionNotFound(args.session_id.clone()))?;

    let tmux_info = session
        .tmux_info
        .ok_or_else(|| CcError::NoTmuxInfo(args.session_id.clone()))?;

    focus_tmux_pane(&tmux_info)?;

    Ok(())
}

/// Focuses the tmux pane specified by TmuxInfo.
/// Runs `tmux select-window` followed by `tmux select-pane`.
fn focus_tmux_pane(info: &TmuxInfo) -> Result<()> {
    // First, select the window
    let window_target = format!("{}:{}", info.session_name, info.window_index);
    let select_window = Command::new("tmux")
        .args(["select-window", "-t", &window_target])
        .output()?;

    if !select_window.status.success() {
        let stderr = String::from_utf8_lossy(&select_window.stderr);
        bail!(
            "Failed to select tmux window '{}': {}",
            window_target,
            stderr.trim()
        );
    }

    // Then, select the pane
    let select_pane = Command::new("tmux")
        .args(["select-pane", "-t", &info.pane_id])
        .output()?;

    if !select_pane.status.success() {
        let stderr = String::from_utf8_lossy(&select_pane.stderr);
        bail!(
            "Failed to select tmux pane '{}': {}",
            info.pane_id,
            stderr.trim()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::{Session, SessionStatus, TmuxInfo};
    use chrono::Utc;
    use std::path::PathBuf;
    use tempfile::TempDir;

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

    fn setup_temp_session_dir() -> TempDir {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        // SAFETY: Tests run serially and this is the only place where we modify this env var
        unsafe {
            std::env::set_var("XDG_CACHE_HOME", temp_dir.path());
        }
        temp_dir
    }

    #[test]
    fn test_session_not_found() {
        let _temp_dir = setup_temp_session_dir();

        let args = FocusArgs {
            session_id: "nonexistent".to_string(),
        };

        let result = run(&args);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("Session not found"));
    }

    #[test]
    fn test_no_tmux_info() {
        let _temp_dir = setup_temp_session_dir();

        // Save a session without tmux info
        let session = create_test_session("no-tmux", None);
        store::save_session(&session).expect("save should succeed");

        let args = FocusArgs {
            session_id: "no-tmux".to_string(),
        };

        let result = run(&args);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("no tmux information"));
    }
}
