use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Args;
use serde::Deserialize;

use super::reminder;
use super::reminder::ReminderSettings;
use crate::commands::agent::store;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReminderHookArgs {}

#[derive(Debug, Default, Deserialize)]
struct ClaudePostToolUseInput {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    tool_input: BashToolInput,
    #[serde(default)]
    tool_response: BashToolResponse,
}

#[derive(Debug, Default, Deserialize)]
struct BashToolInput {
    #[serde(default)]
    command: String,
    #[serde(default)]
    run_in_background: bool,
}

#[derive(Debug, Default, Deserialize)]
struct BashToolResponse {
    #[serde(default, rename = "backgroundTaskId")]
    background_task_id: Option<String>,
}

pub fn run(_args: &ReminderHookArgs) -> Result<Option<i32>> {
    let input: ClaudePostToolUseInput = serde_json::from_reader(io::stdin().lock())
        .context("failed to parse Claude Code PostToolUse input")?;
    if input.tool_name.as_deref() != Some("Bash") || !input.tool_input.run_in_background {
        return Ok(None);
    }

    let settings = ReminderSettings::load();
    let Some(sessions_dir) = settings
        .timeout_for(&input.tool_input.command)
        .map(|_| store::sessions_dir())
        .transpose()?
    else {
        return Ok(None);
    };

    let message = check_task(&input, &settings, &sessions_dir, reminder::sleep)?;
    if let Some(message) = message {
        writeln!(io::stderr().lock(), "{message}")?;
        return Ok(Some(2));
    }

    Ok(None)
}

fn check_task(
    input: &ClaudePostToolUseInput,
    settings: &ReminderSettings,
    sessions_dir: &Path,
    wait: impl FnOnce(Duration),
) -> Result<Option<String>> {
    if input.tool_name.as_deref() != Some("Bash") || !input.tool_input.run_in_background {
        return Ok(None);
    }
    let Some(task_id) = input.tool_response.background_task_id.as_deref() else {
        return Ok(None);
    };
    let Some(timeout) = settings.timeout_for(&input.tool_input.command) else {
        return Ok(None);
    };

    wait(timeout);

    let Some(session) = store::load_session_from(sessions_dir, &input.session_id)? else {
        return Ok(None);
    };
    Ok(session
        .pending_bg_task_ids
        .contains(task_id)
        .then(|| settings.message().to_string()))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use indoc::indoc;
    use rstest::{fixture, rstest};
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::commands::agent::store;
    use crate::commands::agent::types::Session;
    use crate::shared::config::BgTaskReminderConfig;

    #[fixture]
    fn sessions_dir() -> TempDir {
        TempDir::new().expect("temporary sessions directory")
    }

    fn settings() -> ReminderSettings {
        ReminderSettings::from_config(&BgTaskReminderConfig {
            timeout: Some("5m".to_string()),
            message: "Task reminder".to_string(),
            excluded_commands: vec!["sample-review".to_string()],
        })
    }

    fn hook_input(command: &str, run_in_background: bool) -> ClaudePostToolUseInput {
        serde_json::from_value(json!({
            "session_id": "fixture-session",
            "tool_name": "Bash",
            "tool_input": {
                "command": command,
                "run_in_background": run_in_background,
            },
            "tool_response": {
                "backgroundTaskId": "fixture-task-id",
            },
        }))
        .expect("hook input should deserialize")
    }

    fn save_session(sessions_dir: &Path, has_pending_task: bool) {
        let pending_bg_task_ids = if has_pending_task {
            vec!["fixture-task-id"]
        } else {
            Vec::new()
        };
        let session: Session = serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/tmp/fixture",
            "transcript_path": null,
            "tmux_info": null,
            "status": "stopped",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
            "pending_bg_task_ids": pending_bg_task_ids,
        }))
        .expect("session fixture should deserialize");
        store::save_session_to(sessions_dir, &session).expect("session fixture should save");
    }

    #[test]
    fn pending_background_bash_returns_the_configured_message() {
        let sessions_dir = sessions_dir();
        save_session(sessions_dir.path(), true);
        let wait_duration = Cell::new(None);

        let actual = check_task(
            &hook_input("sample-command --watch", true),
            &settings(),
            sessions_dir.path(),
            |duration| wait_duration.set(Some(duration)),
        )
        .unwrap();

        assert_eq!(
            (actual, wait_duration.get()),
            (
                Some("Task reminder".to_string()),
                Some(Duration::from_secs(300))
            )
        );
    }

    #[rstest]
    #[case::foreground_bash(false, "sample-command --watch")]
    #[case::excluded_command(true, "sample-review --wait")]
    fn skipped_commands_return_without_waiting(
        sessions_dir: TempDir,
        #[case] run_in_background: bool,
        #[case] command: &str,
    ) {
        save_session(sessions_dir.path(), true);
        let waits = Cell::new(0);

        let actual = check_task(
            &hook_input(command, run_in_background),
            &settings(),
            sessions_dir.path(),
            |_| waits.set(waits.get() + 1),
        )
        .unwrap();

        assert_eq!((actual, waits.get()), (None, 0));
    }

    #[test]
    fn task_missing_from_the_session_registry_returns_without_a_message() {
        let sessions_dir = sessions_dir();
        save_session(sessions_dir.path(), false);
        let waits = Cell::new(0);

        let actual = check_task(
            &hook_input("sample-command --watch", true),
            &settings(),
            sessions_dir.path(),
            |_| waits.set(waits.get() + 1),
        )
        .unwrap();

        assert_eq!((actual, waits.get()), (None, 1));
    }

    #[test]
    fn background_task_id_is_read_from_claude_tool_response() {
        let input: ClaudePostToolUseInput = serde_json::from_str(indoc! {r#"
            {
              "session_id": "fixture-session",
              "tool_name": "Bash",
              "tool_input": {"command": "sample-command", "run_in_background": true},
              "tool_response": {"backgroundTaskId": "fixture-task-id"}
            }
        "#})
        .expect("hook input should deserialize");

        assert_eq!(
            (
                input.session_id,
                input.tool_name,
                input.tool_input.command,
                input.tool_input.run_in_background,
                input.tool_response.background_task_id,
            ),
            (
                "fixture-session".to_string(),
                Some("Bash".to_string()),
                "sample-command".to_string(),
                true,
                Some("fixture-task-id".to_string()),
            )
        );
    }
}
