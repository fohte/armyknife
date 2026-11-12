use std::io::Write;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::json;

use super::super::store::{
    PermissionRequestDecision, PermissionRequestWait, PermissionRequestWaiter,
};

const POLL_INTERVAL: Duration = Duration::from_millis(100);
// Leave time for Claude Code to fall back to its terminal dialog before the command hook times out.
const MAX_WAIT: Duration = Duration::from_secs(590);

pub(super) fn wait_and_write(
    sessions_dir: &Path,
    session_id: &str,
    agent_key: &str,
    request_id: &str,
    output: impl Write,
) -> Result<()> {
    let deadline = Instant::now() + MAX_WAIT;
    let mut waiter = PermissionRequestWaiter::open(sessions_dir)?;
    wait_until_deadline(
        &mut waiter,
        session_id,
        agent_key,
        request_id,
        deadline,
        output,
    )
}

fn wait_until_deadline(
    waiter: &mut PermissionRequestWaiter,
    session_id: &str,
    agent_key: &str,
    request_id: &str,
    deadline: Instant,
    mut output: impl Write,
) -> Result<()> {
    loop {
        let wait = if Instant::now() >= deadline {
            waiter.expire(session_id, agent_key, request_id)?
        } else {
            waiter.poll(session_id, agent_key, request_id)?
        };
        match wait {
            PermissionRequestWait::Pending => {
                thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())))
            }
            PermissionRequestWait::Finished => return Ok(()),
            PermissionRequestWait::Decision {
                decision,
                tool_input,
                answers,
            } => {
                return write_response(decision, tool_input, answers, &mut output);
            }
        }
    }
}

fn write_response(
    decision: PermissionRequestDecision,
    mut tool_input: serde_json::Value,
    answers: Option<serde_json::Value>,
    mut output: impl Write,
) -> Result<()> {
    let decision = match decision {
        PermissionRequestDecision::Allow => match answers {
            Some(answers) => {
                let Some(updated_input) = tool_input.as_object_mut() else {
                    anyhow::bail!("permission request input must be an object to apply answers");
                };
                updated_input.insert("answers".to_string(), answers);
                json!({ "behavior": "allow", "updatedInput": tool_input })
            }
            None => json!({ "behavior": "allow" }),
        },
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
    fn expired_wait_finishes_without_writing_a_hook_response() {
        let sessions_dir = tempfile::tempdir().expect("sessions directory should be created");
        let session_id = "fixture-session";
        let request_id = "fixture-request";
        let agent_key = crate::commands::agent::types::MAIN_THREAD_AGENT_KEY;
        let mut session: crate::commands::agent::types::Session = serde_json::from_value(json!({
            "session_id": session_id,
            "cwd": "/tmp/fixture",
            "transcript_path": null,
            "tmux_info": null,
            "status": "waiting_input",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .expect("session should deserialize");
        session
            .pending_permission_agent_ids
            .insert(agent_key.to_string());
        session
            .pending_permission_request_ids
            .insert(agent_key.to_string(), request_id.to_string());
        super::super::super::store::save_session_to(sessions_dir.path(), &session)
            .expect("session should be saved");
        let session_lock =
            super::super::super::store::lock_session_for_update(sessions_dir.path(), session_id)
                .expect("session should lock");
        session_lock
            .register_permission_request(
                agent_key,
                request_id,
                Some("Bash"),
                &json!({ "command": "echo fixture" }),
            )
            .expect("permission request should register");
        session_lock
            .save(&session)
            .expect("pending session should save");
        let mut waiter = PermissionRequestWaiter::open(sessions_dir.path())
            .expect("permission request waiter should open");
        let mut output = Vec::new();

        wait_until_deadline(
            &mut waiter,
            session_id,
            agent_key,
            request_id,
            Instant::now() - Duration::from_secs(1),
            &mut output,
        )
        .expect("expired wait should finish successfully");

        assert_eq!(
            (
                output,
                super::super::super::store::permission_request_data(
                    sessions_dir.path(),
                    session_id,
                    request_id,
                )
                .expect("expired request should be readable"),
            ),
            (Vec::<u8>::new(), None),
        );
    }

    #[test]
    fn deny_keeps_the_turn_running() {
        let mut output = Vec::new();
        write_response(
            PermissionRequestDecision::Deny {
                message: "Permission request rejected from watch-web.".to_string(),
                interrupt: false,
            },
            json!({ "command": "echo fictional" }),
            None,
            &mut output,
        )
        .expect("response should be written");

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output).expect("response should be JSON"),
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
    fn allow_approves_the_original_tool_input() {
        let mut output = Vec::new();
        write_response(
            PermissionRequestDecision::Allow,
            json!({ "command": "echo fictional" }),
            None,
            &mut output,
        )
        .expect("response should be written");

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output).expect("response should be JSON"),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": { "behavior": "allow" },
                },
            }),
        );
    }

    #[test]
    fn allow_returns_answers_with_the_original_tool_input() {
        let mut output = Vec::new();
        write_response(
            PermissionRequestDecision::Allow,
            json!({
                "questions": [{ "question": "Which format?" }],
            }),
            Some(json!({ "Which format?": "Markdown" })),
            &mut output,
        )
        .expect("response should be written");

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output).expect("response should be JSON"),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {
                        "behavior": "allow",
                        "updatedInput": {
                            "questions": [{ "question": "Which format?" }],
                            "answers": { "Which format?": "Markdown" },
                        },
                    },
                },
            }),
        );
    }
}
