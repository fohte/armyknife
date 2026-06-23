use anyhow::{Result, bail};
use clap::Args;

use super::types::TMUX_SESSION_OPTION;
use crate::infra::{process, tmux};
use crate::shared::command::find_command_path;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ResumeArgs {
    /// Claude Code session ID to resume. When omitted, the session ID is read from
    /// the current tmux pane's `@armyknife-last-claude-code-session-id` user option.
    pub session_id: Option<String>,
}

/// Runs the resume command.
/// If a session ID argument is provided, resumes that session directly.
/// Otherwise, reads the session ID from the current tmux pane's user option.
pub fn run(args: &ResumeArgs) -> Result<()> {
    let session_id = match args.session_id.as_deref() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => resolve_session_id_from_pane()?,
    };

    // Find claude command path
    let claude_path = find_command_path("claude")
        .ok_or_else(|| anyhow::anyhow!("Could not find 'claude' command in PATH"))?;

    // Replace current process with `claude --resume <session_id>`; only returns on failure.
    let err = process::exec_replace(&claude_path, ["--resume", &session_id]);
    bail!("Failed to exec claude: {}", err)
}

fn resolve_session_id_from_pane() -> Result<String> {
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

    Ok(session_id)
}
