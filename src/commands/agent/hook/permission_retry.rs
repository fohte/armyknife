use std::io::Write;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use serde_json::{Value, json};

use super::super::{
    store,
    types::{Engine, HookEvent},
};

pub(super) fn write_allow_if_approved(
    event: HookEvent,
    engine: Engine,
    sessions_dir: &Path,
    session_id: &str,
    tool_name: Option<&str>,
    tool_input: Option<&Value>,
    mut output: impl Write,
) -> Result<bool> {
    if event != HookEvent::PreToolUse || engine != Engine::Claude {
        return Ok(false);
    }
    let (Some(tool_name), Some(tool_input)) = (tool_name, tool_input) else {
        return Ok(false);
    };
    let approved = store::consume_permission_retry_approval_from(
        sessions_dir,
        session_id,
        tool_name,
        tool_input,
        Utc::now().timestamp(),
    )?;
    if !approved {
        return Ok(false);
    }

    let response = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
        }
    });
    serde_json::to_writer(&mut output, &response)?;
    writeln!(output)?;
    output.flush()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rstest::rstest;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    fn save_approval(sessions_dir: &Path, expires_at: i64) {
        store::insert_permission_retry_approval_from(
            sessions_dir,
            "fixture-session-cedar",
            "Bash",
            &json!({ "command": "printf fixture" }),
            expires_at,
        )
        .expect("approval should be saved");
    }

    #[test]
    fn matching_pre_tool_use_returns_allow_once() {
        let sessions_dir = TempDir::new().expect("temporary sessions directory should be created");
        save_approval(sessions_dir.path(), Utc::now().timestamp() + 60);
        let mut first_output = Vec::new();
        let first_approved = write_allow_if_approved(
            HookEvent::PreToolUse,
            Engine::Claude,
            sessions_dir.path(),
            "fixture-session-cedar",
            Some("Bash"),
            Some(&json!({ "command": "printf fixture" })),
            &mut first_output,
        )
        .expect("matching hook should be handled");
        let mut second_output = Vec::new();
        let second_approved = write_allow_if_approved(
            HookEvent::PreToolUse,
            Engine::Claude,
            sessions_dir.path(),
            "fixture-session-cedar",
            Some("Bash"),
            Some(&json!({ "command": "printf fixture" })),
            &mut second_output,
        )
        .expect("second matching hook should be checked");

        assert_eq!(
            (
                first_approved,
                serde_json::from_slice::<Value>(&first_output)
                    .expect("allow response should be JSON"),
                second_approved,
                second_output,
            ),
            (
                true,
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "allow",
                    },
                }),
                false,
                Vec::<u8>::new(),
            ),
        );
    }

    #[rstest]
    #[case::other_session("fixture-session-maple", "Bash", json!({ "command": "printf fixture" }), Utc::now().timestamp() + 60)]
    #[case::other_tool("fixture-session-cedar", "Read", json!({ "command": "printf fixture" }), Utc::now().timestamp() + 60)]
    #[case::different_input("fixture-session-cedar", "Bash", json!({ "command": "printf changed" }), Utc::now().timestamp() + 60)]
    #[case::expired("fixture-session-cedar", "Bash", json!({ "command": "printf fixture" }), 0)]
    fn nonmatching_hook_does_not_return_allow(
        #[case] session_id: &str,
        #[case] tool_name: &str,
        #[case] tool_input: Value,
        #[case] expires_at: i64,
    ) {
        let sessions_dir = TempDir::new().expect("temporary sessions directory should be created");
        save_approval(sessions_dir.path(), expires_at);
        let mut output = Vec::new();

        assert_eq!(
            (
                write_allow_if_approved(
                    HookEvent::PreToolUse,
                    Engine::Claude,
                    sessions_dir.path(),
                    session_id,
                    Some(tool_name),
                    Some(&tool_input),
                    &mut output,
                )
                .expect("nonmatching hook should be checked"),
                output,
            ),
            (false, Vec::<u8>::new()),
        );
    }
}
