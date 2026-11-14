use std::io::Write;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::json;

use super::super::store;
use super::super::store::{
    PermissionRequestDecision, PermissionRequestWait, PermissionRequestWaiter,
};
use super::super::types::{Engine, MAIN_THREAD_AGENT_KEY};

const POLL_INTERVAL: Duration = Duration::from_millis(100);
// Leave time for a terminal decision before either engine's command hook times out.
const MAX_WAIT: Duration = Duration::from_secs(590);

pub(super) fn wait_for_request(
    engine: Engine,
    agent_id: Option<&str>,
    sessions_dir: &Path,
    session_id: &str,
    output: impl Write,
) -> Result<()> {
    let agent_key = agent_id.unwrap_or(MAIN_THREAD_AGENT_KEY);
    let Some(session) = store::load_session_from(sessions_dir, session_id)? else {
        return Ok(());
    };
    let Some(request_id) = session.pending_permission_request_ids.get(agent_key) else {
        return Ok(());
    };
    wait_and_write(
        engine,
        sessions_dir,
        session_id,
        agent_key,
        request_id,
        output,
    )
}

pub(super) fn wait_and_write(
    engine: Engine,
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
        engine,
        output,
    )
}

fn wait_until_deadline(
    waiter: &mut PermissionRequestWaiter,
    session_id: &str,
    agent_key: &str,
    request_id: &str,
    deadline: Instant,
    engine: Engine,
    mut output: impl Write,
) -> Result<()> {
    loop {
        let wait = if Instant::now() >= deadline {
            let message = format!(
                "No response was provided in watch-web or the terminal within {} seconds. This request was automatically denied.",
                MAX_WAIT.as_secs(),
            );
            waiter.expire_with_deny(session_id, agent_key, request_id, &message)?
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
                return write_response(engine, decision, tool_input, answers, &mut output);
            }
        }
    }
}

fn write_response(
    engine: Engine,
    decision: PermissionRequestDecision,
    mut tool_input: serde_json::Value,
    answers: Option<serde_json::Value>,
    mut output: impl Write,
) -> Result<()> {
    let decision = match decision {
        PermissionRequestDecision::Allow => match answers {
            Some(answers) => {
                if engine == Engine::Codex {
                    anyhow::bail!("Codex PermissionRequest hooks do not support answers");
                }
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

    fn save_pending_permission_request(
        sessions_dir: &Path,
        session_id: &str,
        agent_key: &str,
        request_id: &str,
        tool_input: &serde_json::Value,
    ) {
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
        store::save_session_to(sessions_dir, &session).expect("session should be saved");
        let session_lock =
            store::lock_session_for_update(sessions_dir, session_id).expect("session should lock");
        session_lock
            .register_permission_request(agent_key, request_id, Some("Bash"), tool_input)
            .expect("permission request should register");
        session_lock
            .save(&session)
            .expect("pending session should save");
    }

    #[rstest::rstest]
    #[case::permission_request("Bash")]
    #[case::ask_user_question("AskUserQuestion")]
    fn expired_wait_denies_a_pending_request(#[case] tool_name: &str) {
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
                Some(tool_name),
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
            Engine::Claude,
            &mut output,
        )
        .expect("expired wait should finish successfully");

        let message = format!(
            "No response was provided in watch-web or the terminal within {} seconds. This request was automatically denied.",
            MAX_WAIT.as_secs(),
        );
        assert_eq!(
            (
                serde_json::from_slice::<serde_json::Value>(&output)
                    .expect("deny response should be JSON"),
                super::super::super::store::permission_request_data(
                    sessions_dir.path(),
                    session_id,
                    request_id,
                )
                .expect("expired request should be readable"),
            ),
            (
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PermissionRequest",
                        "decision": {
                            "behavior": "deny",
                            "message": message,
                            "interrupt": false,
                        },
                    },
                }),
                None,
            ),
        );
    }

    #[test]
    fn deny_keeps_the_turn_running() {
        let mut output = Vec::new();
        write_response(
            Engine::Claude,
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
            Engine::Claude,
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
            Engine::Claude,
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

    #[rstest::rstest]
    #[case::allow_main_thread(
        PermissionRequestDecision::Allow,
        json!({ "behavior": "allow" }),
        None,
        MAIN_THREAD_AGENT_KEY,
    )]
    #[case::deny(
        PermissionRequestDecision::Deny {
            message: "Rejected from watch-web.".to_string(),
            interrupt: false,
        },
        json!({
            "behavior": "deny",
            "message": "Rejected from watch-web.",
            "interrupt": false,
        }),
        Some("agent-fixture"),
        "agent-fixture",
    )]
    fn codex_decisions_match_permission_request_hook_output(
        #[case] decision: PermissionRequestDecision,
        #[case] expected_decision: serde_json::Value,
        #[case] agent_id: Option<&str>,
        #[case] agent_key: &str,
    ) {
        let sessions_dir = tempfile::tempdir().expect("sessions directory should be created");
        let session_id = "fixture-session";
        let request_id = "fixture-request";
        save_pending_permission_request(
            sessions_dir.path(),
            session_id,
            agent_key,
            request_id,
            &json!({ "command": "echo fictional" }),
        );
        store::resolve_permission_request(
            sessions_dir.path(),
            session_id,
            agent_key,
            request_id,
            &decision,
        )
        .expect("permission request should resolve");
        let mut output = Vec::new();
        wait_for_request(
            Engine::Codex,
            agent_id,
            sessions_dir.path(),
            session_id,
            &mut output,
        )
        .expect("Codex permission hook should write its decision");

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output)
                .expect("Codex response should be JSON"),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": expected_decision,
                },
            }),
        );
    }

    #[test]
    fn codex_rejects_answer_payloads_that_its_hook_schema_does_not_support() {
        let sessions_dir = tempfile::tempdir().expect("sessions directory should be created");
        let session_id = "fixture-session";
        let request_id = "fixture-request";
        save_pending_permission_request(
            sessions_dir.path(),
            session_id,
            MAIN_THREAD_AGENT_KEY,
            request_id,
            &json!({ "questions": [{ "question": "Which format?" }] }),
        );
        let answers = json!({ "Which format?": "Markdown" });
        store::resolve_permission_request_with_answers(
            sessions_dir.path(),
            session_id,
            MAIN_THREAD_AGENT_KEY,
            request_id,
            &PermissionRequestDecision::Allow,
            Some(&answers),
        )
        .expect("permission request with answers should resolve");
        let mut output = Vec::new();
        let result = wait_for_request(
            Engine::Codex,
            None,
            sessions_dir.path(),
            session_id,
            &mut output,
        );

        assert_eq!(
            (result.err().map(|error| error.to_string()), output,),
            (
                Some("Codex PermissionRequest hooks do not support answers".to_string()),
                Vec::<u8>::new(),
            ),
        );
    }
}
