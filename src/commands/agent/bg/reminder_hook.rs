use std::io::{self, Write};
use std::path::PathBuf;
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
    let settings = ReminderSettings::load();
    run_with(
        &input,
        &settings,
        store::sessions_dir,
        reminder::sleep,
        io::stderr().lock(),
    )
}

fn run_with(
    input: &ClaudePostToolUseInput,
    settings: &ReminderSettings,
    sessions_dir: impl FnOnce() -> Result<PathBuf>,
    wait: impl FnOnce(Duration),
    mut stderr: impl Write,
) -> Result<Option<i32>> {
    let message = check_task(input, settings, sessions_dir, wait)?;
    if let Some(message) = message {
        writeln!(stderr, "{message}")?;
        return Ok(Some(2));
    }

    Ok(None)
}

fn check_task(
    input: &ClaudePostToolUseInput,
    settings: &ReminderSettings,
    sessions_dir: impl FnOnce() -> Result<PathBuf>,
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

    let sessions_dir = sessions_dir()?;
    let Some(session) = store::load_session_from(&sessions_dir, &input.session_id)? else {
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
    use std::path::Path;

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

    #[fixture]
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

    #[rstest]
    fn pending_background_bash_writes_the_message_and_returns_exit_code_2(
        settings: ReminderSettings,
    ) {
        let sessions_dir = sessions_dir();
        save_session(sessions_dir.path(), true);
        let wait_duration = Cell::new(None);
        let mut stderr = Vec::new();

        let exit_code = run_with(
            &hook_input("sample-command --watch", true),
            &settings,
            || Ok(sessions_dir.path().to_path_buf()),
            |duration| wait_duration.set(Some(duration)),
            &mut stderr,
        )
        .unwrap();

        assert_eq!(
            (
                exit_code,
                String::from_utf8(stderr).unwrap(),
                wait_duration.get(),
            ),
            (
                Some(2),
                "Task reminder\n".to_string(),
                Some(Duration::from_secs(300))
            )
        );
    }

    #[rstest]
    #[case::foreground_bash(false, "sample-command --watch")]
    #[case::excluded_command(true, "sample-review --wait")]
    fn skipped_commands_return_without_waiting(
        sessions_dir: TempDir,
        settings: ReminderSettings,
        #[case] run_in_background: bool,
        #[case] command: &str,
    ) {
        save_session(sessions_dir.path(), true);
        let waits = Cell::new(0);
        let mut stderr = Vec::new();

        let actual = run_with(
            &hook_input(command, run_in_background),
            &settings,
            || Ok(sessions_dir.path().to_path_buf()),
            |_| waits.set(waits.get() + 1),
            &mut stderr,
        )
        .unwrap();

        assert_eq!((actual, stderr, waits.get()), (None, Vec::new(), 0));
    }

    #[rstest]
    fn task_missing_from_the_session_registry_returns_without_a_message(
        sessions_dir: TempDir,
        settings: ReminderSettings,
    ) {
        save_session(sessions_dir.path(), true);
        let waits = Cell::new(0);

        let actual = check_task(
            &hook_input("sample-command --watch", true),
            &settings,
            || Ok(sessions_dir.path().to_path_buf()),
            |duration| {
                assert_eq!(duration, Duration::from_secs(300));
                waits.set(waits.get() + 1);
                store::update_session_in(sessions_dir.path(), "fixture-session", |session| {
                    session.pending_bg_task_ids.remove("fixture-task-id")
                })
                .expect("completed task should be removed from the registry");
            },
        )
        .unwrap();

        assert_eq!((actual, waits.get()), (None, 1));
    }

    #[rstest]
    fn background_task_id_is_read_from_claude_tool_response(settings: ReminderSettings) {
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
                input.session_id.as_str(),
                input.tool_name.as_deref(),
                input.tool_input.command.as_str(),
                input.tool_input.run_in_background,
                input.tool_response.background_task_id.as_deref(),
            ),
            (
                "fixture-session",
                Some("Bash"),
                "sample-command",
                true,
                Some("fixture-task-id"),
            )
        );
        let wait_duration = Cell::new(None);
        let mut stderr = Vec::new();
        let exit_code = run_with(
            &input,
            &settings,
            || Ok(PathBuf::from("/missing/session/registry")),
            |duration| wait_duration.set(Some(duration)),
            &mut stderr,
        )
        .unwrap();
        assert_eq!(
            (
                exit_code,
                String::from_utf8(stderr).unwrap(),
                wait_duration.get()
            ),
            (None, String::new(), Some(Duration::from_secs(300)))
        );
    }
}
