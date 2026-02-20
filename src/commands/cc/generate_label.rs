use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::Result;
use clap::Args;

use super::store;

/// Arguments for the hidden `generate-label` subcommand.
///
/// Spawned as a background process by the hook to asynchronously generate
/// a short session label via the Claude CLI, then update the session JSON.
#[derive(Args, Clone, PartialEq, Eq)]
pub struct GenerateLabelArgs {
    /// Session ID to update with the generated label
    #[arg(long)]
    pub session_id: String,

    /// The user prompt text to generate a label from
    #[arg(long)]
    pub prompt: String,

    /// Sessions directory override (for testing)
    #[arg(long)]
    pub sessions_dir: Option<PathBuf>,
}

pub fn run(args: &GenerateLabelArgs) -> Result<()> {
    let sessions_dir = match &args.sessions_dir {
        Some(dir) => dir.clone(),
        None => store::sessions_dir()?,
    };

    let label = generate_label_via_claude(&args.prompt)?;
    update_session_label(&sessions_dir, &args.session_id, &label)?;

    Ok(())
}

/// Calls `claude -p --model haiku` to generate a short label from a prompt.
fn generate_label_via_claude(prompt: &str) -> Result<String> {
    let system_prompt = "Generate a very short (2-5 word) title for this task. \
        Output ONLY the title text, nothing else. No quotes, no punctuation at the end.";

    let output = Command::new("claude")
        .args(["-p", "--model", "haiku", "--system-prompt", system_prompt])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut child| {
            // Write the user prompt to stdin
            if let Some(ref mut stdin) = child.stdin {
                use std::io::Write;
                let _ = stdin.write_all(prompt.as_bytes());
            }
            child.wait_with_output()
        })?;

    let label = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if label.is_empty() {
        return Err(anyhow::anyhow!("claude returned empty label"));
    }

    Ok(label)
}

/// Updates the session JSON file with the generated label.
/// Only sets the label if the session still has no label (avoids overwriting
/// a label that was set by other means between spawn and completion).
fn update_session_label(
    sessions_dir: &std::path::Path,
    session_id: &str,
    label: &str,
) -> Result<()> {
    let mut session = store::load_session_from(sessions_dir, session_id)?
        .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;

    // Only update if label is still unset to avoid race conditions
    if session.label.is_some() {
        return Ok(());
    }

    session.label = Some(label.to_string());
    store::save_session_to(sessions_dir, &session)?;

    Ok(())
}

/// Spawns a background process to generate a label for a session.
///
/// This is called from the hook handler when a `UserPromptSubmit` event is received
/// for a session that has no label. The background process runs independently
/// so the hook returns immediately without blocking Claude Code.
pub fn spawn_label_generation(sessions_dir: &std::path::Path, session_id: &str, prompt: &str) {
    // Use the current binary to run the generate-label subcommand
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(_) => return, // Best-effort: silently skip if we can't find ourselves
    };

    let sessions_dir_str = sessions_dir.to_string_lossy().to_string();

    // Spawn as a fully detached background process
    let _ = Command::new(exe)
        .args([
            "cc",
            "generate-label",
            "--session-id",
            session_id,
            "--prompt",
            prompt,
            "--sessions-dir",
            &sessions_dir_str,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn(); // Ignore errors: label generation is best-effort
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::{Session, SessionStatus};
    use chrono::Utc;
    use rstest::rstest;
    use tempfile::TempDir;

    fn create_test_session(id: &str) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: std::path::PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
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
    fn update_session_label_sets_label_when_none() {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let sessions_dir = temp_dir.path();

        let session = create_test_session("test-label");
        store::save_session_to(sessions_dir, &session).expect("save should succeed");

        update_session_label(sessions_dir, "test-label", "My Task Title")
            .expect("update should succeed");

        let loaded = store::load_session_from(sessions_dir, "test-label")
            .expect("load should succeed")
            .expect("session should exist");
        assert_eq!(loaded.label, Some("My Task Title".to_string()));
    }

    #[rstest]
    fn update_session_label_skips_when_label_already_set() {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let sessions_dir = temp_dir.path();

        let mut session = create_test_session("test-label-existing");
        session.label = Some("Existing Label".to_string());
        store::save_session_to(sessions_dir, &session).expect("save should succeed");

        update_session_label(sessions_dir, "test-label-existing", "New Label")
            .expect("update should succeed");

        let loaded = store::load_session_from(sessions_dir, "test-label-existing")
            .expect("load should succeed")
            .expect("session should exist");
        // Label should remain unchanged
        assert_eq!(loaded.label, Some("Existing Label".to_string()));
    }

    #[rstest]
    fn update_session_label_errors_when_session_not_found() {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let sessions_dir = temp_dir.path();

        let result = update_session_label(sessions_dir, "nonexistent", "Label");
        assert!(result.is_err());
    }
}
