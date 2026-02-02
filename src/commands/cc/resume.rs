use std::os::unix::process::CommandExt;
use std::process::Command;

use anyhow::{Result, bail};
use clap::Args;

use super::types::TMUX_SESSION_OPTION;
use crate::infra::tmux;
use crate::shared::command::find_command_path;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ResumeArgs {}

/// Runs the resume command.
/// Reads the session ID from the current tmux pane's user option and,
/// if set, resumes that Claude Code session.
pub fn run(_args: &ResumeArgs) -> Result<()> {
    let session_id = tmux::get_current_pane_option(TMUX_SESSION_OPTION).ok_or_else(|| {
        anyhow::anyhow!(
            "No Claude Code session ID found for this pane (option '{}' not set)",
            TMUX_SESSION_OPTION
        )
    })?;

    if session_id.is_empty() {
        bail!(
            "No Claude Code session ID found for this pane (option '{}' is empty)",
            TMUX_SESSION_OPTION
        );
    }

    // Clear the pane option before exec, so after session ends the option is clean
    if let Some(pane_id) = tmux::current_pane_id() {
        let _ = tmux::unset_pane_option(&pane_id, TMUX_SESSION_OPTION);
    }

    // Find claude command path
    let claude_path = find_command_path("claude")
        .ok_or_else(|| anyhow::anyhow!("Could not find 'claude' command in PATH"))?;

    // Replace current process with `claude --resume <session_id>`
    let err = Command::new(&claude_path)
        .args(["--resume", &session_id])
        .exec();

    // exec() only returns if there was an error
    bail!("Failed to exec claude: {}", err)
}

#[cfg(test)]
mod tests {
    // Tests for this module require tmux environment which cannot be easily mocked.
    // Integration tests should be done manually in a tmux session.
}
