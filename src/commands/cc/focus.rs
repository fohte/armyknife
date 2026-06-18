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
    let sessions_dir = store::sessions_dir()?;
    let mut session = store::load_session_from(&sessions_dir, &args.session_id)?
        .ok_or_else(|| CcError::SessionNotFound(args.session_id.clone()))?;
    let pane_id = session
        .tmux_info
        .as_ref()
        .map(|info| info.pane_id.clone())
        .ok_or_else(|| CcError::NoTmuxInfo(args.session_id.clone()))?;

    tmux::focus_pane(&pane_id)?;

    if session.status == SessionStatus::Stopped && session.read_at.is_none() {
        session.read_at = Some(Utc::now());
        store::save_session_to(&sessions_dir, &session)?;
    }

    Ok(())
}
