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

    let claude_path = find_command_path("claude")
        .ok_or_else(|| anyhow::anyhow!("Could not find 'claude' command in PATH"))?;

    let err = process::exec_replace(&claude_path, ["--resume", &session_id]);
    bail!("Failed to exec claude: {}", err)
}

fn resolve_session_id_from_pane() -> Result<String> {
    let pane_id = current_pane_id()?;
    tmux::get_pane_option(&pane_id, TMUX_SESSION_OPTION)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No Claude Code session ID found for pane {} (option '{}' not set or empty)",
                pane_id,
                TMUX_SESSION_OPTION
            )
        })
}

/// Returns the tmux pane ID of the caller, read from `$TMUX_PANE`.
///
/// Resolving by `$TMUX_PANE` (set by tmux when it spawns the pane's process)
/// rather than by tmux's notion of the focused pane is required so that resume
/// targets the pane that invoked the command even if the user switches focus
/// before tmux can answer.
fn current_pane_id() -> Result<String> {
    match std::env::var("TMUX_PANE") {
        Ok(value) if !value.is_empty() => Ok(value),
        _ => bail!("Not running inside a tmux pane: $TMUX_PANE is not set"),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::returns_value_when_set(Some("%12"), Ok("%12".to_string()))]
    #[case::errors_when_unset(
        None,
        Err("Not running inside a tmux pane: $TMUX_PANE is not set".to_string())
    )]
    #[case::errors_when_empty(
        Some(""),
        Err("Not running inside a tmux pane: $TMUX_PANE is not set".to_string())
    )]
    fn current_pane_id_cases(
        #[case] env_value: Option<&str>,
        #[case] expected: std::result::Result<String, String>,
    ) {
        temp_env::with_vars([("TMUX_PANE", env_value)], || {
            assert_eq!(current_pane_id().map_err(|e| e.to_string()), expected);
        });
    }
}
