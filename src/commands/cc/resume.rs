use std::os::unix::process::CommandExt;
use std::process::Command;

use anyhow::{Result, bail};
use clap::Args;

use crate::infra::tmux;
use crate::shared::command::find_command_path;

/// Prefix used to identify Claude Code session IDs in pane titles.
const PANE_TITLE_PREFIX: &str = "claude:";

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ResumeArgs {}

/// Runs the resume command.
/// Reads the current tmux pane title and, if it contains a session ID,
/// resumes that Claude Code session.
pub fn run(_args: &ResumeArgs) -> Result<()> {
    let pane_title = tmux::get_current_pane_title()
        .ok_or_else(|| anyhow::anyhow!("Not running inside tmux or could not get pane title"))?;

    let session_id = extract_session_id(&pane_title).ok_or_else(|| {
        anyhow::anyhow!(
            "Pane title '{}' does not contain a Claude Code session ID (expected format: 'claude:<session-id>')",
            pane_title
        )
    })?;

    // Clear the pane title before exec, so after session ends the title is clean
    if let Some(pane_id) = tmux::current_pane_id() {
        let _ = tmux::set_pane_title(&pane_id, "");
    }

    // Find claude command path
    let claude_path = find_command_path("claude")
        .ok_or_else(|| anyhow::anyhow!("Could not find 'claude' command in PATH"))?;

    // Replace current process with `claude --resume <session_id>`
    let err = Command::new(&claude_path)
        .args(["--resume", session_id])
        .exec();

    // exec() only returns if there was an error
    bail!("Failed to exec claude: {}", err)
}

/// Extracts the session ID from a pane title if it starts with "claude:".
/// Returns None if the title doesn't match the expected format.
fn extract_session_id(pane_title: &str) -> Option<&str> {
    pane_title
        .strip_prefix(PANE_TITLE_PREFIX)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::valid_session_id("claude:abc-123-def", Some("abc-123-def"))]
    #[case::uuid_session_id(
        "claude:550e8400-e29b-41d4-a716-446655440000",
        Some("550e8400-e29b-41d4-a716-446655440000")
    )]
    #[case::no_prefix("editor", None)]
    #[case::wrong_prefix("session:abc", None)]
    #[case::empty_session_id("claude:", None)]
    #[case::empty_title("", None)]
    #[case::partial_prefix("claud:abc", None)]
    fn test_extract_session_id(#[case] pane_title: &str, #[case] expected: Option<&str>) {
        assert_eq!(extract_session_id(pane_title), expected);
    }
}
