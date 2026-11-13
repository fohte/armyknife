use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::Json;
use axum::extract::{Path as RoutePath, State};
use indoc::indoc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    ActionState, ApiError, CommandResult, load_session, resolve_sessions_dir, run_blocking,
};
use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session, SessionStatus};
use crate::shared::command;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const OUTPUT_TAIL_BYTES: usize = 8 * 1024;
const PROCESS_CHECK_INTERVAL: Duration = Duration::from_millis(25);

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
    let transcript_path = session
        .transcript_path
        .clone()
        .ok_or_else(|| ApiError::conflict("session has no transcript"))?;
    let expected_message = request.message_text;
    let expected_commands = request.commands;
    let found = run_blocking(move || {
        transcript_contains_commands(&transcript_path, &expected_message, &expected_commands)
    })
    .await?;
    if !found {
        return Err(ApiError::conflict(
            "the selected command is not present in the agent transcript",
        ));
    }
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

    let sessions_dir = if session.engine == Engine::Claude {
        Some(resolve_sessions_dir(state.sessions_dir).map_err(ApiError::internal)?)
    } else {
        None
    };
    let marker = if let Some(sessions_dir) = sessions_dir.as_ref() {
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
    let result_message =
        super::messages::watch_web_message_content(&session, marker.as_deref(), &result_message);
    let runtime = state.runtime;
    run_blocking(move || {
        super::messages::deliver_user_message(
            runtime.as_ref(),
            &session,
            sessions_dir.as_deref(),
            &result_message,
            &queued_content,
        )
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

pub(super) fn execute_command(cwd: &Path, command: &str) -> Result<CommandResult> {
    let shell = std::env::var_os("SHELL").context("SHELL is not set")?;
    let mut process = command::new(shell);
    process
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        process.process_group(0);
    }
    let mut child = process.spawn().context("failed to start login shell")?;
    let stdout = child
        .stdout
        .take()
        .context("shell stdout was not captured")?;
    let stderr = child
        .stderr
        .take()
        .context("shell stderr was not captured")?;
    let stdout_reader = thread::spawn(move || read_tail(stdout, OUTPUT_TAIL_BYTES));
    let stderr_reader = thread::spawn(move || read_tail(stderr, OUTPUT_TAIL_BYTES));
    let (status, timed_out) = wait_for_process(COMMAND_TIMEOUT, |timed_out| {
        if timed_out {
            terminate_process(&mut child);
            child
                .wait()
                .context("failed to stop timed out login shell")
                .map(Some)
        } else {
            child.try_wait().context("failed to wait for login shell")
        }
    })?;
    terminate_process(&mut child);
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader panicked"))??;

    Ok(CommandResult {
        exit_code: if timed_out {
            124
        } else {
            status.code().unwrap_or(1)
        },
        stdout,
        stderr,
        timed_out,
    })
}

fn wait_for_process<T>(
    timeout: Duration,
    mut poll: impl FnMut(bool) -> Result<Option<T>>,
) -> Result<(T, bool)> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = poll(false)? {
            return Ok((status, false));
        }
        if Instant::now() >= deadline {
            return poll(true)?
                .map(|status| (status, true))
                .ok_or_else(|| anyhow::anyhow!("timed out process was not stopped"));
        }
        thread::sleep(PROCESS_CHECK_INTERVAL);
    }
}

fn terminate_process(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let process_group = -(child.id() as i32);
        // The shell may have started child processes that still hold output pipes open.
        let _ = unsafe { libc::kill(process_group, libc::SIGKILL) };
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

fn read_tail(mut reader: impl Read, limit: usize) -> Result<String> {
    let mut tail = VecDeque::with_capacity(limit);
    let mut buffer = [0; 4096];
    let mut total_bytes = 0usize;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total_bytes = total_bytes.saturating_add(read);
        for byte in &buffer[..read] {
            if limit == 0 {
                continue;
            }
            if tail.len() == limit {
                tail.pop_front();
            }
            tail.push_back(*byte);
        }
    }
    let truncated = total_bytes > limit;
    let output = String::from_utf8_lossy(&tail.into_iter().collect::<Vec<_>>()).into_owned();
    Ok(if truncated {
        format!("[earlier output omitted]\n{output}")
    } else {
        output
    })
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

fn transcript_contains_commands(
    path: &Path,
    expected_message: &str,
    expected_commands: &[String],
) -> Result<bool> {
    let reader = open_transcript(path)?;
    for line in reader.lines() {
        let line = line?;
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(message) = assistant_text(&record)
            && message == expected_message
            && commands_are_candidates(&message, expected_commands)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn commands_are_candidates(message: &str, commands: &[String]) -> bool {
    let mut remaining = HashMap::<String, usize>::new();
    for candidate in bang_commands(message) {
        *remaining.entry(candidate).or_default() += 1;
    }
    commands.iter().all(|command| {
        let Some(available) = remaining.get_mut(command) else {
            return false;
        };
        if *available == 0 {
            return false;
        }
        *available -= 1;
        true
    })
}

fn open_transcript(path: &Path) -> Result<Box<dyn BufRead>> {
    let is_compressed = path.extension().and_then(|extension| extension.to_str()) == Some("zst");
    if is_compressed {
        let file =
            File::open(path).with_context(|| format!("open transcript {}", path.display()))?;
        return Ok(Box::new(BufReader::new(zstd::stream::read::Decoder::new(
            file,
        )?)));
    }
    match File::open(path) {
        Ok(file) => Ok(Box::new(BufReader::new(file))),
        Err(_) => {
            let compressed_path = compressed_path(path);
            let compressed = File::open(&compressed_path)
                .with_context(|| format!("open transcript {}", compressed_path.display()))?;
            Ok(Box::new(BufReader::new(zstd::stream::read::Decoder::new(
                compressed,
            )?)))
        }
    }
}

fn compressed_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".zst");
    PathBuf::from(name)
}

fn assistant_text(record: &Value) -> Option<String> {
    let content = if record.get("type").and_then(Value::as_str) == Some("assistant") {
        record.get("message")?.get("content")?
    } else if record.get("type").and_then(Value::as_str) == Some("response_item")
        && record.pointer("/payload/type")?.as_str() == Some("message")
        && record.pointer("/payload/role")?.as_str() == Some("assistant")
    {
        record.pointer("/payload/content")?
    } else {
        return None;
    };
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => Some(
            blocks
                .iter()
                .filter(|block| {
                    matches!(
                        block.get("type").and_then(Value::as_str),
                        Some("text" | "input_text" | "output_text")
                    )
                })
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

fn bang_commands(message: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in message.lines() {
        let trimmed = line.trim_start();
        let fence_prefix = trimmed
            .chars()
            .next()
            .filter(|character| *character == '`' || *character == '~');
        if let Some(character) = fence_prefix {
            let length = trimmed
                .chars()
                .take_while(|value| *value == character)
                .count();
            if length >= 3 {
                match fence {
                    Some((open_character, open_length))
                        if open_character == character && length >= open_length =>
                    {
                        fence = None;
                    }
                    None => fence = Some((character, length)),
                    _ => {}
                }
                continue;
            }
        }
        if fence.is_some() {
            if let Some(command) = line.strip_prefix("! ") {
                let command = command.trim();
                if !command.is_empty() {
                    commands.push(command.to_string());
                }
            }
        } else {
            commands.extend(inline_bang_commands(line));
        }
    }
    commands
}

fn inline_bang_commands(line: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut index = 0;
    let bytes = line.as_bytes();
    while index < bytes.len() {
        if bytes[index] != b'`' {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index] == b'`' {
            index += 1;
        }
        let delimiter_length = index - start;
        let mut close = index;
        let mut found = None;
        while close < bytes.len() {
            if bytes[close] != b'`' {
                close += 1;
                continue;
            }
            let close_start = close;
            while close < bytes.len() && bytes[close] == b'`' {
                close += 1;
            }
            if close - close_start == delimiter_length {
                found = Some(close_start);
                break;
            }
        }
        if let Some(end) = found {
            let code = line.get(index..end).unwrap_or_default();
            if let Some(command) = code.strip_prefix("! ") {
                let command = command.trim();
                if !command.is_empty() {
                    commands.push(command.to_string());
                }
            }
            index += delimiter_length;
        }
    }
    commands
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::time::Duration;

    use indoc::indoc;
    use rstest::rstest;

    use super::{
        assistant_text, bang_commands, command_script, commands_are_candidates, read_tail,
        wait_for_process,
    };

    #[rstest]
    #[case::fenced_commands(indoc! {"\
        ```sh
        ! printf first
        ! printf second
        ```"}, vec!["printf first", "printf second"])]
    #[case::inline_command("Run `! printf result` now.", vec!["printf result"])]
    #[case::other_code_is_ignored("Run `printf result` and use ! plain text.", vec![])]
    fn detects_only_commands_in_markdown_code(#[case] message: &str, #[case] expected: Vec<&str>) {
        assert_eq!(bang_commands(message), expected);
    }

    #[rstest]
    #[case::claude(serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"hello"}]}}), Some("hello".to_string()))]
    #[case::codex(serde_json::json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}}), Some("hello".to_string()))]
    #[case::user(serde_json::json!({"type":"user","message":{"content":"hello"}}), None)]
    fn extracts_assistant_message(
        #[case] record: serde_json::Value,
        #[case] expected: Option<String>,
    ) {
        assert_eq!(assistant_text(&record), expected);
    }

    #[rstest]
    #[case::authored_command("Run `! echo safe`.", vec!["echo safe"], true)]
    #[case::unlisted_command("Run `! echo safe`.", vec!["echo unsafe"], false)]
    #[case::duplicate_request("Run `! echo safe`.", vec!["echo safe", "echo safe"], false)]
    fn validates_requested_command_occurrences(
        #[case] message: &str,
        #[case] commands: Vec<&str>,
        #[case] expected: bool,
    ) {
        let commands = commands.into_iter().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(commands_are_candidates(message, &commands), expected);
    }

    #[rstest]
    #[case::within_limit(b"result", 8, "result")]
    #[case::exact_limit(b"result", 6, "result")]
    #[case::over_limit(b"first second", 6, "[earlier output omitted]\nsecond")]
    #[case::zero_limit(b"result", 0, "[earlier output omitted]\n")]
    fn keeps_only_the_tail_when_output_exceeds_limit(
        #[case] input: &[u8],
        #[case] limit: usize,
        #[case] expected: &str,
    ) {
        assert_eq!(read_tail(Cursor::new(input), limit).unwrap(), expected);
    }

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

    #[rstest]
    #[case::finished(Some(23), (23, false), false)]
    #[case::timed_out(None, (124, true), true)]
    fn stops_only_when_process_exceeds_the_deadline(
        #[case] observed_status: Option<i32>,
        #[case] expected: (i32, bool),
        #[case] stop_expected: bool,
    ) {
        let mut stopped = false;
        let result = wait_for_process(Duration::ZERO, |timed_out| {
            if timed_out {
                stopped = true;
                Ok(Some(124))
            } else {
                Ok(observed_status)
            }
        })
        .unwrap();

        assert_eq!((result, stopped), (expected, stop_expected));
    }
}
