use std::path::PathBuf;

use axum::http::StatusCode;
use indoc::indoc;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt;

use super::super::CommandResult;
use super::{ActionContext, action_context, post, response_snapshot, session};
use crate::commands::agent::types::Engine;

#[rstest]
#[tokio::test]
async fn command_preview_shows_cwd_only_for_a_transcript_command(action_context: ActionContext) {
    let transcript_path = action_context.temp_dir.path().join("transcript.jsonl");
    std::fs::write(
        &transcript_path,
        r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Run `! printf safe` now."}]}}"#,
    )
    .unwrap();
    let mut session = session("sample-session");
    session.engine = Engine::Codex;
    session.cwd = action_context.temp_dir.path().join("workspace");
    session.transcript_path = Some(transcript_path);
    std::fs::create_dir(&session.cwd).unwrap();
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/commands/preview",
            json!({
                "messageText": "Run `! printf safe` now.",
                "commands": ["printf safe"],
            }),
        ))
        .await
        .unwrap();
    let body = response_snapshot(response, &action_context).await;

    assert_eq!(
        body,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": {"cwd": session.cwd.display().to_string()},
            "calls": [],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn command_execution_rejects_commands_absent_from_the_transcript(
    action_context: ActionContext,
) {
    let transcript_path = action_context.temp_dir.path().join("transcript.jsonl");
    std::fs::write(
        &transcript_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Run `! printf safe` now."}]}}"#,
    )
    .unwrap();
    let mut session = session("sample-session");
    session.engine = Engine::Codex;
    session.cwd = PathBuf::from(action_context.temp_dir.path());
    session.transcript_path = Some(transcript_path);
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/commands/execute",
            json!({
                "messageText": "Run `! printf safe` now.",
                "commands": ["printf unsafe"],
            }),
        ))
        .await
        .unwrap();
    let body = response_snapshot(response, &action_context).await;

    assert_eq!(
        body,
        json!({
            "status": StatusCode::CONFLICT.as_u16(),
            "body": {"error": "the selected command is not present in the agent transcript"},
            "calls": [],
        }),
    );
}

#[rstest]
#[case::claude(Engine::Claude)]
#[case::codex(Engine::Codex)]
#[tokio::test]
async fn command_execution_returns_output_to_the_session(
    #[case] engine: Engine,
    action_context: ActionContext,
) {
    let transcript_path = action_context.temp_dir.path().join("transcript.jsonl");
    let message = indoc! {"
        ```sh
        ! printf safe
        ```
    "}
    .trim_end();
    let record = json!({
        "type": "assistant",
        "message": {"content": [{"type": "text", "text": message}]},
    });
    std::fs::write(&transcript_path, record.to_string()).unwrap();
    let mut session = session("sample-session");
    session.engine = engine;
    session.cwd = PathBuf::from(action_context.temp_dir.path());
    session.transcript_path = Some(transcript_path);
    action_context.save_session(&session);

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/commands/execute",
            json!({
                "messageText": message,
                "commands": ["printf safe"],
            }),
        ))
        .await
        .unwrap();
    let body = response_snapshot(response, &action_context).await;
    let has_marker = crate::commands::agent::store::watch_web_user_marker_from(
        &action_context.temp_dir.path().join("sessions"),
        "sample-session",
    )
    .unwrap()
    .is_some();
    let cwd = action_context.temp_dir.path().display().to_string();
    let expected_message = indoc! {r#"
        Command exited with code 0 in {cwd}.

        Command:
        ```
        $ printf safe
        ```

        stdout:
        ```
        fixture output
        ```

        stderr:
        ```
        (empty)
        ```
    "#}
    .replace("{cwd}", &cwd);

    assert_eq!(
        (body, has_marker),
        (
            json!({
                "status": StatusCode::OK.as_u16(),
                "body": {"exitCode": 0, "succeeded": true, "timedOut": false},
                "calls": [
                    json!({
                        "kind": "execute-command",
                        "cwd": action_context.temp_dir.path(),
                        "command": indoc::indoc! {"
                            set +e
                            __armyknife_bang_exit_code=0
                            printf safe
                            __armyknife_bang_command_exit_code=$?
                            if [ \"$__armyknife_bang_command_exit_code\" -ne 0 ] && [ \"$__armyknife_bang_exit_code\" -eq 0 ]; then __armyknife_bang_exit_code=$__armyknife_bang_command_exit_code; fi
                            exit \"$__armyknife_bang_exit_code\"
                        "}
                        .trim_end(),
                    }),
                    json!({
                        "kind": "message",
                        "sessionId": "sample-session",
                        "content": expected_message.trim_end(),
                    }),
                ],
            }),
            engine == Engine::Claude,
        ),
    );
}

#[rstest]
#[tokio::test]
async fn timed_out_command_sends_partial_output_and_reports_timeout(action_context: ActionContext) {
    let transcript_path = action_context.temp_dir.path().join("transcript.jsonl");
    let message = "Run `! printf safe` now.";
    let record = json!({
        "type": "assistant",
        "message": {"content": [{"type": "text", "text": message}]},
    });
    std::fs::write(&transcript_path, record.to_string()).unwrap();
    let mut session = session("sample-session");
    session.engine = Engine::Codex;
    session.cwd = PathBuf::from(action_context.temp_dir.path());
    session.transcript_path = Some(transcript_path);
    action_context.save_session(&session);
    action_context.set_command_result(CommandResult {
        exit_code: 124,
        stdout: "partial output".to_string(),
        stderr: "command error".to_string(),
        timed_out: true,
    });

    let response = action_context
        .router()
        .oneshot(post(
            "/sessions/sample-session/commands/execute",
            json!({
                "messageText": message,
                "commands": ["printf safe"],
            }),
        ))
        .await
        .unwrap();
    let body = response_snapshot(response, &action_context).await;
    let cwd = action_context.temp_dir.path().display().to_string();
    let expected_message = indoc! {r#"
        Command timed out after 30 seconds in {cwd}.

        Command:
        ```
        $ printf safe
        ```

        stdout:
        ```
        partial output
        ```

        stderr:
        ```
        command error
        ```
    "#}
    .replace("{cwd}", &cwd);

    assert_eq!(
        body,
        json!({
            "status": StatusCode::OK.as_u16(),
            "body": {"exitCode": 124, "succeeded": false, "timedOut": true},
            "calls": [
                json!({
                    "kind": "execute-command",
                    "cwd": action_context.temp_dir.path(),
                    "command": indoc! {r#"
                        set +e
                        __armyknife_bang_exit_code=0
                        printf safe
                        __armyknife_bang_command_exit_code=$?
                        if [ "$__armyknife_bang_command_exit_code" -ne 0 ] && [ "$__armyknife_bang_exit_code" -eq 0 ]; then __armyknife_bang_exit_code=$__armyknife_bang_command_exit_code; fi
                        exit "$__armyknife_bang_exit_code"
                    "#}
                    .trim_end(),
                }),
                json!({
                    "kind": "message",
                    "sessionId": "sample-session",
                    "content": expected_message.trim_end(),
                }),
            ],
        }),
    );
}
