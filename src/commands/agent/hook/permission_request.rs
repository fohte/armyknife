use std::io::Write;
use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use serde_json::{Value, json};

use super::super::store::{
    PermissionRequestDecision, PermissionRequestWait, PermissionRequestWaiter,
};

const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(super) fn wait_and_write(
    sessions_dir: &Path,
    session_id: &str,
    agent_key: &str,
    request_id: &str,
    output: impl Write,
) -> Result<()> {
    let mut waiter = PermissionRequestWaiter::open(sessions_dir)?;
    loop {
        match waiter.poll(session_id, agent_key, request_id)? {
            PermissionRequestWait::Pending => thread::sleep(POLL_INTERVAL),
            PermissionRequestWait::Finished => return Ok(()),
            PermissionRequestWait::Decision(decision) => {
                return write_response(decision, output);
            }
        }
    }
}

fn write_response(decision: PermissionRequestDecision, mut output: impl Write) -> Result<()> {
    let decision = match decision {
        PermissionRequestDecision::Allow { updated_input } => {
            let mut decision =
                serde_json::Map::from_iter([("behavior".to_string(), json!("allow"))]);
            if let Some(updated_input) = updated_input {
                decision.insert("updatedInput".to_string(), updated_input);
            }
            Value::Object(decision)
        }
        PermissionRequestDecision::Deny { message, interrupt } => json!({
            "behavior": "deny",
            "message": message,
            "interrupt": interrupt,
        }),
    };
    let response = json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": decision,
        }
    });
    serde_json::to_writer(&mut output, &response)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_keeps_the_turn_running() {
        let mut output = Vec::new();
        write_response(
            PermissionRequestDecision::Deny {
                message: "Permission request rejected from watch-web.".to_string(),
                interrupt: false,
            },
            &mut output,
        )
        .expect("response should be written");

        assert_eq!(
            serde_json::from_slice::<Value>(&output).expect("response should be JSON"),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {
                        "behavior": "deny",
                        "message": "Permission request rejected from watch-web.",
                        "interrupt": false,
                    },
                },
            }),
        );
    }

    #[test]
    fn allow_without_updated_input_approves_the_original_tool_input() {
        let mut output = Vec::new();
        write_response(
            PermissionRequestDecision::Allow {
                updated_input: None,
            },
            &mut output,
        )
        .expect("response should be written");

        assert_eq!(
            serde_json::from_slice::<Value>(&output).expect("response should be JSON"),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": { "behavior": "allow" },
                },
            }),
        );
    }

    #[test]
    fn allow_returns_updated_question_answers() {
        let mut output = Vec::new();
        write_response(
            PermissionRequestDecision::Allow {
                updated_input: Some(json!({
                    "questions": [{ "question": "Which display mode?" }],
                    "answers": { "Which display mode?": "Option A" },
                })),
            },
            &mut output,
        )
        .expect("response should be written");

        assert_eq!(
            serde_json::from_slice::<Value>(&output).expect("response should be JSON"),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {
                        "behavior": "allow",
                        "updatedInput": {
                            "questions": [{ "question": "Which display mode?" }],
                            "answers": { "Which display mode?": "Option A" },
                        },
                    },
                },
            }),
        );
    }
}
