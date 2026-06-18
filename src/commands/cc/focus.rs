use anyhow::Result;
use chrono::Utc;
use clap::Args;

use super::error::CcError;
use super::store;
use super::types::SessionStatus;
use crate::infra::tmux;

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
    let pane_id = session
        .tmux_info
        .as_ref()
        .map(|info| info.pane_id.clone())
        .ok_or_else(|| CcError::NoTmuxInfo(args.session_id.clone()))?;

    tmux::focus_pane(&pane_id)?;

    if session.status == SessionStatus::Stopped {
        store::mark_session_read(&args.session_id, Utc::now())?;
    }

    Ok(())
}
