use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use chrono::Utc;
use serde_json::Value;

use super::super::resurrect::{PaneResumeSession, resume_command_for};
use super::super::store;
use super::super::types::{Engine, Session, SessionStatus};
use crate::commands::agent::codex_steer;
use crate::commands::agent::signal::{LibcSignalSender, SignalSender};
use crate::infra::process::ProcessSnapshot;
use crate::infra::tmux;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RewindMode {
    ConversationAndCode,
    Conversation,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ForkedCodexThread {
    pub(super) thread_id: String,
    pub(super) transcript_path: PathBuf,
}

pub(super) trait Host: Send + Sync {
    fn stop_agent(&self, session: &Session) -> Result<()>;
    fn rewind_claude_files(&self, session: &Session, message_id: &str) -> Result<()>;
    fn fork_codex_thread(
        &self,
        session: &Session,
        before_turn_id: &str,
    ) -> Result<ForkedCodexThread>;
    fn respawn_pane(&self, session: &Session, new_session: &Session) -> Result<()>;
}

pub(super) struct LiveHost;

impl Host for LiveHost {
    fn stop_agent(&self, session: &Session) -> Result<()> {
        let Some(tmux_info) = session.tmux_info.as_ref() else {
            bail!("session has no tmux pane");
        };
        let pane_pid =
            tmux::get_pane_pid_for_socket(tmux_info.socket_path.as_deref(), &tmux_info.pane_id)?
                .context("session pane is no longer available")?;
        let snapshot = ProcessSnapshot::capture().context("could not inspect running processes")?;
        let Some(agent_pid) = snapshot.find_self_or_descendant_by_command(
            pane_pid,
            session.engine.process_name(),
            64,
        ) else {
            return Ok(());
        };
        if let Err(error) = LibcSignalSender.send(agent_pid, libc::SIGTERM)
            && error.raw_os_error() != Some(libc::ESRCH)
        {
            return Err(error)
                .with_context(|| format!("could not stop {:?} process", session.engine));
        }
        ensure!(
            crate::infra::process::wait_for_process_exit(agent_pid, Duration::from_secs(5)),
            "{:?} process did not exit after SIGTERM",
            session.engine
        );
        Ok(())
    }

    fn rewind_claude_files(&self, session: &Session, message_id: &str) -> Result<()> {
        let output = claude_file_rewind_command(session, message_id)
            .output()
            .context("failed to start Claude Code file rewind")?;
        ensure!(
            output.status.success(),
            "Claude Code file rewind exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(())
    }

    fn fork_codex_thread(
        &self,
        session: &Session,
        before_turn_id: &str,
    ) -> Result<ForkedCodexThread> {
        let (thread_id, transcript_path) =
            codex_steer::fork_thread(&session.session_id, before_turn_id)?;
        Ok(ForkedCodexThread {
            thread_id,
            transcript_path,
        })
    }

    fn respawn_pane(&self, session: &Session, new_session: &Session) -> Result<()> {
        let Some(tmux_info) = session.tmux_info.as_ref() else {
            bail!("session has no tmux pane");
        };
        tmux::set_pane_option_for_socket(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
            super::super::types::TMUX_SESSION_OPTION,
            &new_session.session_id,
        )?;
        let pane_session = PaneResumeSession {
            session_id: new_session.session_id.clone(),
            ancestor_session_ids: new_session.ancestor_session_ids.clone(),
            engine: Some(new_session.engine),
            launched_with_vibe: new_session.launched_with_vibe,
        };
        let command = resume_command_for(&pane_session, new_session.engine, false)
            .context("could not build the session resume command")?;
        tmux::respawn_pane_for_socket(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
            &command,
        )?;
        Ok(())
    }
}

fn claude_file_rewind_command(session: &Session, message_id: &str) -> std::process::Command {
    let mut command = crate::shared::command::new("claude");
    command
        .current_dir(&session.cwd)
        .env("CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING", "true")
        .args([
            "-p",
            "--resume",
            &session.session_id,
            "--rewind-files",
            message_id,
        ]);
    command
}

pub(super) fn can_rewind(session: &Session) -> bool {
    session.status == SessionStatus::WaitingInput && session.tmux_info.is_some()
}

pub(super) fn rewind_session(
    sessions_dir: &Path,
    session: &Session,
    message_id: &str,
    mode: RewindMode,
    host: &dyn Host,
) -> Result<Session> {
    ensure!(!message_id.trim().is_empty(), "message ID is required");
    let session = store::load_session_from(sessions_dir, &session.session_id)?
        .context("session no longer exists")?;
    ensure!(can_rewind(&session), "session is not waiting for input");
    ensure!(
        session.status == SessionStatus::WaitingInput && !session.sweep_signaled,
        "session is not waiting for input"
    );
    if session.engine == Engine::Codex {
        ensure!(
            mode == RewindMode::Conversation,
            "Codex only supports conversation rewind"
        );
    }
    if session.engine == Engine::Claude {
        let source = session
            .transcript_path
            .as_deref()
            .context("Claude Code session has no transcript path")?;
        validate_claude_user_message(source, message_id)?;
    }

    let session = mark_rewind_started(sessions_dir, &session.session_id)?;
    if let Err(error) = host.stop_agent(&session) {
        if let Err(marker_error) = clear_rewind_marker(sessions_dir, &session.session_id) {
            return Err(error).context(format!(
                "could not clear the rewind marker after stop failed: {marker_error:#}"
            ));
        }
        return Err(error);
    }

    let (session_id, transcript_path) = match session.engine {
        Engine::Claude => {
            let operation = (|| {
                let (fork_id, transcript_path) = create_claude_fork(
                    &session,
                    message_id,
                    matches!(
                        mode,
                        RewindMode::ConversationAndCode | RewindMode::Conversation
                    ),
                )?;
                if matches!(mode, RewindMode::ConversationAndCode | RewindMode::Code)
                    && let Err(error) = host.rewind_claude_files(&session, message_id)
                {
                    fs::remove_file(&transcript_path)
                        .context("failed to remove the incomplete Claude Code fork transcript")?;
                    return Err(error);
                }
                Ok((fork_id, transcript_path))
            })();
            match operation {
                Ok(fork) => fork,
                Err(error) => {
                    pause_session_after_rewind_failure(sessions_dir, &session.session_id, &error)?;
                    return Err(error);
                }
            }
        }
        Engine::Codex => match host.fork_codex_thread(&session, message_id) {
            Ok(fork) => (fork.thread_id, fork.transcript_path),
            Err(error) => {
                pause_session_after_rewind_failure(sessions_dir, &session.session_id, &error)?;
                return Err(error);
            }
        },
    };

    let latest_session = match store::load_session_from(sessions_dir, &session.session_id) {
        Ok(Some(latest)) => latest,
        Ok(None) => {
            let error = anyhow::anyhow!("session no longer exists after stopping the agent");
            pause_session_after_rewind_failure(sessions_dir, &session.session_id, &error)?;
            return Err(error);
        }
        Err(error) => {
            pause_session_after_rewind_failure(sessions_dir, &session.session_id, &error)?;
            return Err(error);
        }
    };
    let now = Utc::now();
    let mut stopped_session = latest_session.clone();
    stopped_session.status = SessionStatus::Paused;
    stopped_session.updated_at = now;
    stopped_session.read_at = None;
    stopped_session.sweep_signaled = false;

    let mut forked_session = latest_session;
    forked_session.session_id = session_id;
    forked_session.transcript_path = Some(transcript_path);
    forked_session.status = SessionStatus::Stopped;
    forked_session.created_at = now;
    forked_session.updated_at = now;
    forked_session.read_at = None;
    forked_session.agent_status = None;
    forked_session.agent_status_clear_pending = false;
    forked_session.current_tool = None;
    forked_session.pending_bg_task_ids.clear();
    forked_session.pending_agent_task_ids.clear();
    forked_session.pending_permission_agent_ids.clear();
    forked_session.pending_permission_request_ids.clear();
    forked_session.pending_human_review_ids.clear();
    forked_session
        .ancestor_session_ids
        .push(session.session_id.clone());
    forked_session.sweep_signaled = false;

    if let Err(error) = store::save_rewind_sessions(sessions_dir, &stopped_session, &forked_session)
    {
        if session.engine == Engine::Claude
            && let Some(transcript_path) = forked_session.transcript_path.as_deref()
        {
            let _ = fs::remove_file(transcript_path);
        }
        pause_session_after_rewind_failure(sessions_dir, &session.session_id, &error)?;
        return Err(error);
    }
    if let Err(error) = host.respawn_pane(&session, &forked_session) {
        tracing::error!(
            event = "agent.watch_web.rewind.respawn_failed",
            original_session_id = %session.session_id,
            forked_session_id = %forked_session.session_id,
            error = %error,
        );
        return Err(error);
    }
    Ok(forked_session)
}

fn mark_rewind_started(sessions_dir: &Path, session_id: &str) -> Result<Session> {
    let mut marked_session = None;
    store::update_session_in(sessions_dir, session_id, |session| {
        if !can_rewind(session) || session.sweep_signaled {
            return false;
        }
        session.sweep_signaled = true;
        session.updated_at = Utc::now();
        marked_session = Some(session.clone());
        true
    })?;
    marked_session.context("session is not waiting for input")
}

fn clear_rewind_marker(sessions_dir: &Path, session_id: &str) -> Result<()> {
    store::update_session_in(sessions_dir, session_id, |session| {
        if !session.sweep_signaled {
            return false;
        }
        session.sweep_signaled = false;
        session.updated_at = Utc::now();
        true
    })
}

fn pause_session_after_rewind_failure(
    sessions_dir: &Path,
    session_id: &str,
    operation_error: &anyhow::Error,
) -> Result<()> {
    store::update_session_in(sessions_dir, session_id, |session| {
        if session.status == SessionStatus::Paused && !session.sweep_signaled {
            return false;
        }
        session.status = SessionStatus::Paused;
        session.sweep_signaled = false;
        session.updated_at = Utc::now();
        true
    })
    .with_context(|| {
        format!("could not preserve the paused session after rewind failed: {operation_error:#}")
    })
}

fn create_claude_fork(
    session: &Session,
    message_id: &str,
    truncate_before_message: bool,
) -> Result<(String, PathBuf)> {
    let source = session
        .transcript_path
        .as_deref()
        .context("Claude Code session has no transcript path")?;
    let fork_id = uuid::Uuid::new_v4().to_string();
    let target = source
        .parent()
        .context("Claude Code transcript has no parent directory")?
        .join(format!("{fork_id}.jsonl"));
    write_claude_fork(
        source,
        &target,
        message_id,
        &fork_id,
        truncate_before_message,
    )?;
    Ok((fork_id, target))
}

fn validate_claude_user_message(source: &Path, message_id: &str) -> Result<()> {
    let source_file = fs::File::open(source)
        .with_context(|| format!("failed to open Claude Code transcript {}", source.display()))?;
    for line in BufReader::new(source_file).lines() {
        let line = line?;
        let record: Value = serde_json::from_str(&line)
            .with_context(|| format!("invalid JSON in transcript {}", source.display()))?;
        if record.get("uuid").and_then(Value::as_str) == Some(message_id) {
            ensure!(
                record.get("type").and_then(Value::as_str) == Some("user"),
                "selected message is not a user message"
            );
            return Ok(());
        }
    }
    bail!("selected message is not in the Claude Code transcript")
}

fn write_claude_fork(
    source: &Path,
    target: &Path,
    message_id: &str,
    fork_id: &str,
    truncate_before_message: bool,
) -> Result<()> {
    validate_claude_user_message(source, message_id)?;
    let source_file = fs::File::open(source)
        .with_context(|| format!("failed to open Claude Code transcript {}", source.display()))?;
    let mut target_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .with_context(|| {
            format!(
                "failed to create Claude Code transcript {}",
                target.display()
            )
        })?;
    let result = (|| {
        for line in BufReader::new(source_file).lines() {
            let line = line?;
            let mut record: Value = serde_json::from_str(&line)
                .with_context(|| format!("invalid JSON in transcript {}", source.display()))?;
            if record.get("uuid").and_then(Value::as_str) == Some(message_id)
                && truncate_before_message
            {
                break;
            }
            if let Some(session_id) = record.get_mut("sessionId") {
                *session_id = Value::String(fork_id.to_owned());
            }
            serde_json::to_writer(&mut target_file, &record)?;
            target_file.write_all(b"\n")?;
        }
        target_file.flush()?;
        Ok(())
    })();
    if result.is_err() {
        drop(target_file);
        let _ = fs::remove_file(target);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use std::sync::Mutex;
    use tempfile::TempDir;

    struct FakeHost {
        calls: Mutex<Vec<Value>>,
    }

    impl FakeHost {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl Host for FakeHost {
        fn stop_agent(&self, session: &Session) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(serde_json::json!({"stop": session.session_id}));
            Ok(())
        }

        fn rewind_claude_files(&self, session: &Session, message_id: &str) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(serde_json::json!({"rewindFiles": [session.session_id, message_id]}));
            Ok(())
        }

        fn fork_codex_thread(
            &self,
            _session: &Session,
            _before_turn_id: &str,
        ) -> Result<ForkedCodexThread> {
            bail!("unexpected Codex fork")
        }

        fn respawn_pane(&self, session: &Session, new_session: &Session) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(serde_json::json!({"respawn": [session.session_id, new_session.session_id]}));
            Ok(())
        }
    }

    #[test]
    fn cuts_claude_transcript_before_the_selected_user_message_and_keeps_source_unchanged() {
        let directory = TempDir::new().unwrap();
        let source = directory.path().join("original.jsonl");
        let target = directory.path().join("fork.jsonl");
        let original = indoc! {r#"
            {"uuid":"fixture-before","sessionId":"fixture-old","type":"user","message":{"content":"Earlier request."}}
            {"uuid":"fixture-selected","sessionId":"fixture-old","type":"user","message":{"content":"Selected request."}}
            {"uuid":"fixture-after","sessionId":"fixture-old","type":"assistant","message":{"content":[{"type":"text","text":"Later response."}]}}
        "#};
        fs::write(&source, original).unwrap();

        write_claude_fork(&source, &target, "fixture-selected", "fixture-new", true).unwrap();

        let target_records = fs::read_to_string(&target)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            (fs::read_to_string(&source).unwrap(), target_records),
            (
                original.to_string(),
                vec![serde_json::json!({
                    "uuid": "fixture-before",
                    "sessionId": "fixture-new",
                    "type": "user",
                    "message": {"content": "Earlier request."}
                })],
            ),
        );
    }

    #[test]
    fn claude_file_rewind_enables_sdk_checkpointing_without_starting_a_process() {
        let session = test_session();
        let command = claude_file_rewind_command(&session, "fixture-message-id");
        let actual = (
            command
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            command.get_current_dir().map(Path::to_path_buf),
            command
                .get_envs()
                .map(|(name, value)| {
                    (
                        name.to_string_lossy().into_owned(),
                        value.map(|value| value.to_string_lossy().into_owned()),
                    )
                })
                .collect::<Vec<_>>(),
        );

        assert_eq!(
            actual,
            (
                vec![
                    "-p",
                    "--resume",
                    "fixture-session",
                    "--rewind-files",
                    "fixture-message-id",
                ]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>(),
                Some(PathBuf::from("/tmp/fixture")),
                vec![(
                    "CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING".to_string(),
                    Some("true".to_string()),
                )],
            ),
        );
    }

    #[test]
    fn code_only_rewinds_files_and_keeps_the_full_conversation() {
        let directory = TempDir::new().unwrap();
        let source = directory.path().join("original.jsonl");
        let original = indoc! {r#"
            {"uuid":"fixture-before","sessionId":"fixture-session","type":"user","message":{"content":"Earlier request."}}
            {"uuid":"fixture-selected","sessionId":"fixture-session","type":"user","message":{"content":"Selected request."}}
            {"uuid":"fixture-after","sessionId":"fixture-session","type":"assistant","message":{"content":[{"type":"text","text":"Later response."}]}}
        "#};
        fs::write(&source, original).unwrap();
        let mut session = test_session();
        session.transcript_path = Some(source.clone());
        store::save_session_to(directory.path(), &session).unwrap();
        let host = FakeHost::new();

        let forked = rewind_session(
            directory.path(),
            &session,
            "fixture-selected",
            RewindMode::Code,
            &host,
        )
        .unwrap();

        let fork_records = fs::read_to_string(forked.transcript_path.as_ref().unwrap())
            .unwrap()
            .lines()
            .map(|line| {
                let mut record: Value = serde_json::from_str(line).unwrap();
                record["sessionId"] = Value::String("fixture-fork".to_string());
                record
            })
            .collect::<Vec<_>>();
        let mut calls = host.calls.lock().unwrap().clone();
        calls[2]["respawn"][1] = Value::String("fixture-fork".to_string());

        let previous = store::load_session_from(directory.path(), &session.session_id)
            .unwrap()
            .unwrap();

        assert_eq!(
            (
                forked.session_id != session.session_id,
                forked.status,
                previous.status,
                previous.sweep_signaled,
                forked.ancestor_session_ids,
                fs::read_to_string(source).unwrap(),
                fork_records,
                calls,
            ),
            (
                true,
                SessionStatus::Stopped,
                SessionStatus::Paused,
                false,
                vec![
                    "fixture-ancestor".to_string(),
                    "fixture-session".to_string()
                ],
                original.to_string(),
                vec![
                    serde_json::json!({
                        "uuid": "fixture-before",
                        "sessionId": "fixture-fork",
                        "type": "user",
                        "message": {"content": "Earlier request."}
                    }),
                    serde_json::json!({
                        "uuid": "fixture-selected",
                        "sessionId": "fixture-fork",
                        "type": "user",
                        "message": {"content": "Selected request."}
                    }),
                    serde_json::json!({
                        "uuid": "fixture-after",
                        "sessionId": "fixture-fork",
                        "type": "assistant",
                        "message": {"content": [{"type": "text", "text": "Later response."}]}
                    }),
                ],
                vec![
                    serde_json::json!({"stop": "fixture-session"}),
                    serde_json::json!({"rewindFiles": ["fixture-session", "fixture-selected"]}),
                    serde_json::json!({"respawn": ["fixture-session", "fixture-fork"]}),
                ],
            ),
        );
    }

    #[rstest::rstest]
    #[case::conversation_and_code(RewindMode::ConversationAndCode, true)]
    #[case::conversation_only(RewindMode::Conversation, false)]
    fn conversation_modes_cut_history_before_the_selected_message(
        #[case] mode: RewindMode,
        #[case] rewind_files: bool,
    ) {
        let directory = TempDir::new().unwrap();
        let source = directory.path().join("original.jsonl");
        let original = indoc! {r#"
            {"uuid":"fixture-before","sessionId":"fixture-session","type":"user","message":{"content":"Earlier request."}}
            {"uuid":"fixture-selected","sessionId":"fixture-session","type":"user","message":{"content":"Selected request."}}
            {"uuid":"fixture-after","sessionId":"fixture-session","type":"assistant","message":{"content":[{"type":"text","text":"Later response."}]}}
        "#};
        fs::write(&source, original).unwrap();
        let mut session = test_session();
        session.transcript_path = Some(source.clone());
        store::save_session_to(directory.path(), &session).unwrap();
        let host = FakeHost::new();

        let forked =
            rewind_session(directory.path(), &session, "fixture-selected", mode, &host).unwrap();

        let fork_records = fs::read_to_string(forked.transcript_path.as_ref().unwrap())
            .unwrap()
            .lines()
            .map(|line| {
                let mut record: Value = serde_json::from_str(line).unwrap();
                record["sessionId"] = Value::String("fixture-fork".to_string());
                record
            })
            .collect::<Vec<_>>();
        let mut calls = host.calls.lock().unwrap().clone();
        if rewind_files {
            calls[2]["respawn"][1] = Value::String("fixture-fork".to_string());
        } else {
            calls[1]["respawn"][1] = Value::String("fixture-fork".to_string());
        }
        let mut expected_calls = vec![serde_json::json!({"stop": "fixture-session"})];
        if rewind_files {
            expected_calls
                .push(serde_json::json!({"rewindFiles": ["fixture-session", "fixture-selected"]}));
        }
        expected_calls.push(serde_json::json!({"respawn": ["fixture-session", "fixture-fork"]}));

        assert_eq!(
            (
                fs::read_to_string(&source).unwrap(),
                fork_records,
                calls,
                store::load_session_from(directory.path(), &session.session_id)
                    .unwrap()
                    .unwrap()
                    .status,
            ),
            (
                original.to_string(),
                vec![serde_json::json!({
                    "uuid": "fixture-before",
                    "sessionId": "fixture-fork",
                    "type": "user",
                    "message": {"content": "Earlier request."}
                })],
                expected_calls,
                SessionStatus::Paused,
            ),
        );
    }

    #[test]
    fn rejects_a_transcript_record_that_is_not_a_user_message() {
        let directory = TempDir::new().unwrap();
        let source = directory.path().join("original.jsonl");
        fs::write(
            &source,
            r#"{"uuid":"fixture-selected","sessionId":"fixture-session","type":"assistant"}"#,
        )
        .unwrap();

        assert_eq!(
            validate_claude_user_message(&source, "fixture-selected")
                .err()
                .map(|error| error.to_string()),
            Some("selected message is not a user message".to_string()),
        );
    }

    #[rstest::rstest]
    #[case::running(SessionStatus::Running)]
    #[case::stopped(SessionStatus::Stopped)]
    #[case::paused(SessionStatus::Paused)]
    #[case::ended(SessionStatus::Ended)]
    fn rejects_rewind_when_session_is_not_waiting(#[case] status: SessionStatus) {
        let mut session = test_session();
        session.status = status;
        let directory = TempDir::new().unwrap();
        store::save_session_to(directory.path(), &session).unwrap();
        let host = FakeHost::new();

        let result = rewind_session(
            directory.path(),
            &session,
            "fixture-message-id",
            RewindMode::Conversation,
            &host,
        );

        assert_eq!(
            (
                result.err().map(|error| error.to_string()),
                host.calls.lock().unwrap().clone()
            ),
            (
                Some("session is not waiting for input".to_string()),
                Vec::new()
            ),
        );
    }

    #[test]
    fn rejects_rewind_when_the_stored_session_started_after_it_was_loaded() {
        let directory = TempDir::new().unwrap();
        let session = test_session();
        store::save_session_to(directory.path(), &session).unwrap();
        let mut running_session = session.clone();
        running_session.status = SessionStatus::Running;
        store::save_session_to(directory.path(), &running_session).unwrap();
        let host = FakeHost::new();

        let result = rewind_session(
            directory.path(),
            &session,
            "fixture-message-id",
            RewindMode::Conversation,
            &host,
        );

        assert_eq!(
            (
                result.err().map(|error| error.to_string()),
                host.calls.lock().unwrap().clone(),
                store::load_session_from(directory.path(), &session.session_id)
                    .unwrap()
                    .unwrap()
                    .status,
            ),
            (
                Some("session is not waiting for input".to_string()),
                Vec::new(),
                SessionStatus::Running,
            ),
        );
    }

    fn test_session() -> Session {
        Session {
            session_id: "fixture-session".to_string(),
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp/fixture"),
            transcript_path: None,
            tty: None,
            tmux_info: Some(crate::commands::agent::types::TmuxInfo {
                session_name: "fixture".to_string(),
                window_name: "fixture".to_string(),
                window_index: 0,
                pane_id: "%7".to_string(),
                socket_path: Some("/tmp/fixture.sock".to_string()),
            }),
            status: SessionStatus::WaitingInput,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: Some("Fixture session".to_string()),
            work_type: None,
            vibe_enabled: false,
            launched_with_vibe: false,
            work_type_pinned: false,
            child_launch: Default::default(),
            ancestor_session_ids: vec!["fixture-ancestor".to_string()],
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }
}
