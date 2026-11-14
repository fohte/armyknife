use std::path::Path;

use anyhow::Result;
use axum::Json;
use axum::extract::{Path as RoutePath, State};
use indoc::indoc;
use serde::{Deserialize, Serialize};

use super::{
    ActionState, ApiError, CommandResult, load_session, resolve_sessions_dir, run_blocking,
};
use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session, SessionStatus};

mod process;
mod transcript;

use process::COMMAND_TIMEOUT;
pub(super) use process::execute_command;
use transcript::transcript_contains_commands;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExecuteRequest {
    message_text: String,
    commands: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExecuteResponse {
    exit_code: i32,
    succeeded: bool,
    timed_out: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PreviewResponse {
    cwd: String,
}

pub(super) async fn preview(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<ExecuteRequest>,
) -> Result<Json<PreviewResponse>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    if session.status == SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }
    validate_request(&request)?;
    validate_transcript(&session, &request).await?;
    Ok(Json(PreviewResponse {
        cwd: session.cwd.display().to_string(),
    }))
}

pub(super) async fn execute(
    State(state): State<ActionState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    let session = load_session(state.clone(), id).await?;
    if session.status == SessionStatus::Ended {
        return Err(ApiError::conflict("session has ended"));
    }
    validate_request(&request)?;

    let lock_key = (
        session.session_id.clone(),
        request.message_text.clone(),
        request.commands.clone(),
    );
    let lock = state.bang_command_locks.get_or_create(lock_key).await;
    let _guard = lock
        .try_lock_owned()
        .map_err(|_| ApiError::conflict("this command is already running"))?;

    validate_transcript(&session, &request).await?;

    let sessions_dir =
        resolve_sessions_dir(state.sessions_dir.clone()).map_err(ApiError::internal)?;
    let marker = if session.engine == Engine::Claude {
        let sessions_dir = sessions_dir.clone();
        let session_id = session.session_id.clone();
        Some(
            run_blocking(move || {
                store::ensure_watch_web_user_marker_from(&sessions_dir, &session_id)
            })
            .await?,
        )
    } else {
        None
    };

    let command = command_script(&request.commands);
    let cwd = session.cwd.clone();
    let runtime = state.runtime.clone();
    let result = run_blocking(move || runtime.execute_command(&cwd, &command)).await?;
    let result_message = command_result_message(&request.commands, &session.cwd, &result);
    let queued_content = result_message.clone();
    super::messages::send_queued_prompt_best_effort(super::messages::QueuedPromptDelivery {
        runtime: state.runtime,
        sessions_dir,
        session,
        marker,
        queued_content,
        message_content: result_message,
        queued_attachments: Vec::new(),
        attachment_paths: Vec::new(),
    })
    .await?;

    Ok(Json(ExecuteResponse {
        exit_code: result.exit_code,
        succeeded: result.exit_code == 0 && !result.timed_out,
        timed_out: result.timed_out,
    }))
}

fn validate_request(request: &ExecuteRequest) -> Result<(), ApiError> {
    if request.message_text.trim().is_empty()
        || request.commands.is_empty()
        || request.commands.len() > 32
        || request
            .commands
            .iter()
            .any(|command| command.trim().is_empty())
    {
        return Err(ApiError::bad_request(
            "messageText and 1 to 32 non-empty commands are required",
        ));
    }
    Ok(())
}

fn command_script(commands: &[String]) -> String {
    let mut script = indoc! {"
        set +e
        __armyknife_bang_exit_code=0
    "}
    .to_string();
    for command in commands {
        script.push_str(command);
        script.push('\n');
        script.push_str(indoc! {"
            __armyknife_bang_command_exit_code=$?
            if [ \"$__armyknife_bang_command_exit_code\" -ne 0 ] && [ \"$__armyknife_bang_exit_code\" -eq 0 ]; then __armyknife_bang_exit_code=$__armyknife_bang_command_exit_code; fi
        "});
    }
    script.push_str("exit \"$__armyknife_bang_exit_code\"");
    script
}

async fn validate_transcript(session: &Session, request: &ExecuteRequest) -> Result<(), ApiError> {
    let transcript_path = session
        .transcript_path
        .clone()
        .ok_or_else(|| ApiError::conflict("session has no transcript"))?;
    let expected_message = request.message_text.clone();
    let expected_commands = request.commands.clone();
    let found = run_blocking(move || {
        transcript_contains_commands(&transcript_path, &expected_message, &expected_commands)
    })
    .await?;
    if found {
        Ok(())
    } else {
        Err(ApiError::conflict(
            "the selected command is not present in the agent transcript",
        ))
    }
}

fn command_result_message(commands: &[String], cwd: &Path, result: &CommandResult) -> String {
    let status = if result.timed_out {
        format!("timed out after {} seconds", COMMAND_TIMEOUT.as_secs())
    } else {
        format!("exited with code {}", result.exit_code)
    };
    let command_lines = commands
        .iter()
        .map(|command| format!("$ {command}"))
        .collect::<Vec<_>>()
        .join("\n");
    let stdout = if result.stdout.is_empty() {
        "(empty)"
    } else {
        &result.stdout
    };
    let stderr = if result.stderr.is_empty() {
        "(empty)"
    } else {
        &result.stderr
    };
    format!(
        indoc! {"
            Command {} in {}.

            Command:
            {}

            stdout:
            {}

            stderr:
            {}
        "},
        status,
        cwd.display(),
        markdown_code_block(&command_lines),
        markdown_code_block(stdout),
        markdown_code_block(stderr),
    )
    .trim_end_matches('\n')
    .to_string()
}

fn markdown_code_block(content: &str) -> String {
    let longest_backtick_run = content
        .as_bytes()
        .split(|byte| *byte != b'`')
        .map(<[u8]>::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(3.max(longest_backtick_run + 1));
    let mut block = String::with_capacity(fence.len() * 2 + content.len() + 2);
    block.push_str(&fence);
    block.push('\n');
    block.push_str(content);
    block.push('\n');
    block.push_str(&fence);
    block
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::command_script;

    #[test]
    fn command_script_runs_every_command_and_preserves_the_first_failure() {
        let script = command_script(&["printf first".to_string(), "printf second".to_string()]);

        assert_eq!(
            script,
            indoc! {"
                set +e
                __armyknife_bang_exit_code=0
                printf first
                __armyknife_bang_command_exit_code=$?
                if [ \"$__armyknife_bang_command_exit_code\" -ne 0 ] && [ \"$__armyknife_bang_exit_code\" -eq 0 ]; then __armyknife_bang_exit_code=$__armyknife_bang_command_exit_code; fi
                printf second
                __armyknife_bang_command_exit_code=$?
                if [ \"$__armyknife_bang_command_exit_code\" -ne 0 ] && [ \"$__armyknife_bang_exit_code\" -eq 0 ]; then __armyknife_bang_exit_code=$__armyknife_bang_command_exit_code; fi
                exit \"$__armyknife_bang_exit_code\"
            "}
            .trim_end(),
        );
    }
}
