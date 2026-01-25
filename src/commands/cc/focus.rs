use anyhow::Result;
use clap::Args;

use super::error::CcError;
use super::store;
use super::types::TmuxInfo;
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

    let tmux_info = session
        .tmux_info
        .ok_or_else(|| CcError::NoTmuxInfo(args.session_id.clone()))?;

    focus_tmux_pane(&tmux_info)?;

    Ok(())
}

/// Focuses the tmux pane specified by TmuxInfo.
/// Switches to the target session, then selects the window and pane.
fn focus_tmux_pane(info: &TmuxInfo) -> Result<()> {
    tmux::switch_to_session(&info.session_name)?;
    let window_target = format!("{}:{}", info.session_name, info.window_index);
    tmux::select_window(&window_target)?;
    tmux::select_pane(&info.pane_id)?;
    Ok(())
}
