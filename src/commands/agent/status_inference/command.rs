use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, ensure};
use chrono::{DateTime, Utc};
use clap::Args;

use super::super::types::{AgentStatus, Engine, Session};
use super::super::{codex_steer, status, store};
use super::fork::infer_claude_status;
use super::prompt::{build_codex_prompt, build_prompt};
#[cfg(not(test))]
use crate::infra::process;
use crate::shared::config::{self, AgentConfig};

const SNAPSHOT_FILE_PREFIX: &str = "armyknife-status-inference-";

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub(crate) struct CodexStopSnapshot {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub last_assistant_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusInferenceSpawnRequest {
    pub session_id: String,
    pub stop_updated_at: DateTime<Utc>,
    pub codex_snapshot: Option<CodexStopSnapshot>,
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct StatusInferenceArgs {
    /// Session whose Stop event requested status inference.
    #[arg(long)]
    pub session: String,

    /// Session-record timestamp captured by that Stop event.
    #[arg(long, value_parser = parse_updated_at)]
    pub updated_at: DateTime<Utc>,

    /// File containing the Codex Stop turn snapshot.
    #[arg(long)]
    pub codex_snapshot_file: Option<PathBuf>,
}

fn parse_updated_at(value: &str) -> std::result::Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|date_time| date_time.with_timezone(&Utc))
        .map_err(|error| format!("invalid RFC 3339 timestamp: {error}"))
}

#[cfg(not(test))]
pub fn spawn_in_background(request: &StatusInferenceSpawnRequest) -> Result<()> {
    let executable = std::env::current_exe().context("failed to resolve current exe")?;
    spawn_in_background_with(
        request,
        &executable,
        &std::env::temp_dir(),
        |executable, args| {
            process::spawn_detached(executable, args, None, &[("ARMYKNIFE_SKIP_HOOKS", "1")])
        },
    )
}

fn spawn_in_background_with<Spawn>(
    request: &StatusInferenceSpawnRequest,
    executable: &Path,
    temp_dir: &Path,
    spawn: Spawn,
) -> Result<()>
where
    Spawn: FnOnce(&Path, &[OsString]) -> std::io::Result<()>,
{
    let snapshot_path = request
        .codex_snapshot
        .as_ref()
        .map(|snapshot| persist_codex_snapshot(snapshot, temp_dir))
        .transpose()?;
    let updated_at = request.stop_updated_at.to_rfc3339();
    let mut args = vec![
        OsString::from("agent"),
        OsString::from("status-inference"),
        OsString::from("--session"),
        OsString::from(&request.session_id),
        OsString::from("--updated-at"),
        OsString::from(updated_at),
    ];
    if let Some(path) = &snapshot_path {
        args.push(OsString::from("--codex-snapshot-file"));
        args.push(path.as_os_str().to_os_string());
    }

    match spawn(executable, &args) {
        Ok(()) => {
            tracing::info!(
                event = "agent.hook.status_inference_spawned",
                session = %request.session_id,
            );
            Ok(())
        }
        Err(error) => {
            if let Some(path) = snapshot_path {
                let _ = std::fs::remove_file(path);
            }
            Err(anyhow!(error).context("failed to spawn status inference worker"))
        }
    }
}

fn persist_codex_snapshot(snapshot: &CodexStopSnapshot, temp_dir: &Path) -> Result<PathBuf> {
    let encoded = serde_json::to_vec(snapshot).context("serialize Codex Stop snapshot")?;
    let mut file = tempfile::Builder::new()
        .prefix(SNAPSHOT_FILE_PREFIX)
        .tempfile_in(temp_dir)
        .context("create Codex Stop snapshot file")?;
    file.write_all(&encoded)
        .context("write Codex Stop snapshot file")?;
    file.flush().context("flush Codex Stop snapshot file")?;
    let (_file, path) = file
        .keep()
        .map_err(|error| anyhow!("persist Codex Stop snapshot file: {error}"))?;
    Ok(path)
}

pub fn run(args: &StatusInferenceArgs) -> Result<()> {
    if let Err(error) = run_inner(args) {
        tracing::warn!(
            event = "agent.status_inference.failed",
            session = %args.session,
            error = %format!("{error:#}"),
        );
    }
    Ok(())
}

fn run_inner(args: &StatusInferenceArgs) -> Result<()> {
    let agent_config = config::load_config_or_default().agent;
    let sessions_dir = store::sessions_dir()?;
    let codex_snapshot = args
        .codex_snapshot_file
        .as_deref()
        .map(read_codex_snapshot)
        .transpose();

    run_inner_with(
        args,
        &sessions_dir,
        &agent_config,
        codex_snapshot,
        infer_claude_status,
        |session, snapshot, prompt, configured_kinds| {
            let last_assistant_message = snapshot
                .last_assistant_message
                .as_deref()
                .filter(|message| !message.trim().is_empty())
                .context("Codex Stop snapshot has no final assistant message")?;
            let request = codex_steer::status_inference::Request {
                session,
                thread_id: &snapshot.thread_id,
                turn_id: snapshot.turn_id.as_deref(),
                last_assistant_message,
                prompt,
                configured_kinds,
            };
            codex_steer::status_inference::infer_status(&request)
        },
    )
}

fn read_codex_snapshot(path: &Path) -> Result<CodexStopSnapshot> {
    let temp_dir = std::env::temp_dir();
    let is_owned_snapshot = path.parent() == Some(temp_dir.as_path())
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(SNAPSHOT_FILE_PREFIX));
    let bytes = std::fs::read(path)
        .with_context(|| format!("read Codex Stop snapshot file {}", path.display()));
    if is_owned_snapshot {
        let _ = std::fs::remove_file(path);
    }
    let bytes = bytes?;
    serde_json::from_slice(&bytes).context("parse Codex Stop snapshot file")
}

fn run_inner_with<ClaudeInference, CodexInference>(
    args: &StatusInferenceArgs,
    sessions_dir: &Path,
    agent_config: &AgentConfig,
    codex_snapshot: Result<Option<CodexStopSnapshot>>,
    infer_claude: ClaudeInference,
    infer_codex: CodexInference,
) -> Result<()>
where
    ClaudeInference: FnOnce(&Session, &str) -> Result<AgentStatus>,
    CodexInference: FnOnce(&Session, &CodexStopSnapshot, &str, &[String]) -> Result<AgentStatus>,
{
    let Some(session) = store::load_session_from(sessions_dir, &args.session)? else {
        tracing::info!(
            event = "agent.status_inference.skipped",
            session = %args.session,
            reason = "session_missing",
        );
        return Ok(());
    };
    if session.agent_status.is_some() || session.updated_at != args.updated_at {
        tracing::info!(
            event = "agent.status_inference.skipped",
            session = %args.session,
            reason = "session_changed_before_inference",
        );
        return Ok(());
    }

    match session.engine {
        Engine::Claude => {
            let Some(configured_prompt) = agent_config.configured_status_inference_prompt() else {
                tracing::info!(
                    event = "agent.status_inference.skipped",
                    session = %args.session,
                    reason = "prompt_not_configured",
                );
                return Ok(());
            };
            let prompt = build_prompt(configured_prompt, &session, agent_config);
            let inferred_status = infer_claude(&session, &prompt)?;
            write_inferred_status(sessions_dir, args, agent_config, inferred_status)?;
        }
        Engine::Codex => {
            let Some(codex_config) = agent_config.configured_codex_status_inference() else {
                tracing::info!(
                    event = "agent.status_inference.skipped",
                    session = %args.session,
                    reason = "codex_inference_disabled",
                );
                return Ok(());
            };
            let Some(configured_prompt) = agent_config.configured_status_inference_prompt() else {
                tracing::info!(
                    event = "agent.status_inference.skipped",
                    session = %args.session,
                    reason = "prompt_not_configured",
                );
                return Ok(());
            };
            let mut configured_kinds = agent_config.statuses.keys().cloned().collect::<Vec<_>>();
            configured_kinds.sort();
            let inference_result = (|| {
                let snapshot = codex_snapshot?.context("Codex Stop snapshot is missing")?;
                let _last_assistant_message = snapshot
                    .last_assistant_message
                    .as_deref()
                    .filter(|message| !message.trim().is_empty())
                    .context("Codex Stop snapshot has no final assistant message")?;
                ensure!(
                    !snapshot.thread_id.is_empty(),
                    "Codex Stop snapshot has no thread_id"
                );
                let prompt = build_codex_prompt(configured_prompt, &session, agent_config);
                infer_codex(&session, &snapshot, &prompt, &configured_kinds)
            })();

            let inferred_status = match inference_result {
                Ok(status) => status,
                Err(error) => {
                    record_codex_failure_status_if_unchanged(
                        sessions_dir,
                        args,
                        agent_config,
                        &format!("{error:#}"),
                    )?;
                    return Ok(());
                }
            };

            if codex_config.shadow_mode {
                status::with_inference_candidate_if_unchanged(
                    sessions_dir,
                    &args.session,
                    args.updated_at,
                    || {
                        tracing::info!(
                            event = "agent.status_inference.shadow_result",
                            session = %args.session,
                            kind = %inferred_status.kind,
                            note = %inferred_status.note,
                        );
                    },
                )?;
                return Ok(());
            }

            write_inferred_status(sessions_dir, args, agent_config, inferred_status)?;
        }
    }
    Ok(())
}

fn write_inferred_status(
    sessions_dir: &Path,
    args: &StatusInferenceArgs,
    agent_config: &AgentConfig,
    inferred_status: AgentStatus,
) -> Result<()> {
    match status::set_inferred_status_if_unchanged(
        sessions_dir,
        &args.session,
        args.updated_at,
        agent_config,
        inferred_status,
    )? {
        true => tracing::info!(
            event = "agent.status_inference.status_set",
            session = %args.session,
        ),
        false => tracing::info!(
            event = "agent.status_inference.status_discarded",
            session = %args.session,
            reason = "session_changed_during_inference",
        ),
    }
    Ok(())
}

pub(crate) fn record_codex_failure_status_if_unchanged(
    sessions_dir: &Path,
    args: &StatusInferenceArgs,
    agent_config: &AgentConfig,
    cause: &str,
) -> Result<bool> {
    let Some(config) = agent_config.configured_codex_status_inference() else {
        return Ok(false);
    };
    tracing::warn!(
        event = "agent.status_inference.failed",
        session = %args.session,
        error = cause,
    );
    let fallback = config
        .failure_status
        .as_ref()
        .context("Codex inference failure status is not configured")?;
    if config.shadow_mode {
        status::with_inference_candidate_if_unchanged(
            sessions_dir,
            &args.session,
            args.updated_at,
            || {
                tracing::info!(
                    event = "agent.status_inference.shadow_failure_candidate",
                    session = %args.session,
                    kind = %fallback.kind,
                    note = %fallback.note,
                    error = cause,
                );
            },
        )?;
        return Ok(false);
    }

    let status_value = AgentStatus {
        kind: fallback.kind.clone(),
        note: fallback.note.clone(),
    };
    let recorded = status::set_inferred_status_if_unchanged(
        sessions_dir,
        &args.session,
        args.updated_at,
        agent_config,
        status_value,
    )?;
    tracing::info!(
        event = if recorded {
            "agent.status_inference.failure_status_set"
        } else {
            "agent.status_inference.failure_status_discarded"
        },
        session = %args.session,
        reason = if recorded {
            "inference_failed"
        } else {
            "session_changed"
        },
    );
    Ok(recorded)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::io::{self, Write};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;
    use tracing_subscriber::prelude::*;

    use super::*;
    use crate::commands::agent::types::SessionStatus;
    use crate::shared::config::{CodexStatusInferenceConfig, StatusInferenceStatusConfig};

    struct TraceBufferWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for TraceBufferWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let mut output = self
                .0
                .lock()
                .map_err(|_| io::Error::other("trace buffer lock was poisoned"))?;
            output.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn capture_info_json_events<T>(operation: impl FnOnce() -> T) -> (T, Vec<serde_json::Value>) {
        let log_output = Arc::new(Mutex::new(Vec::new()));
        let writer_output = log_output.clone();
        let fmt_layer = tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .without_time()
            .with_level(false)
            .with_target(false)
            .with_writer(move || TraceBufferWriter(writer_output.clone()))
            .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                *metadata.level() == tracing::Level::INFO
            }));
        let subscriber = tracing_subscriber::registry().with(fmt_layer);
        let result = tracing::subscriber::with_default(subscriber, operation);
        let log_output = log_output.lock().expect("log output lock").clone();
        let log_events = String::from_utf8(log_output)
            .expect("trace output should be UTF-8")
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<Vec<serde_json::Value>, _>>()
            .expect("trace output should contain JSON events");

        (result, log_events)
    }

    struct InferenceWorkerContext {
        _temp_dir: TempDir,
        sessions_dir: PathBuf,
        session: Session,
        agent_config: AgentConfig,
    }

    #[fixture]
    fn inference_worker_context() -> InferenceWorkerContext {
        let temp_dir = TempDir::new().expect("temp directory should be created");
        let sessions_dir = temp_dir.path().join("armyknife/cc/sessions");
        let session = Session {
            archived_at: None,
            pr_context: None,
            session_id: "fixture-session".to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp/fixture"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
        };
        store::save_session_to(&sessions_dir, &session).expect("session should be saved");
        let agent_config: AgentConfig = serde_yaml::from_str(indoc! {
            "
            status_inference_prompt: Classify why the session stopped.
            statuses:
              needs_user: { state: user }
              external_wait: { state: waiting }
              finished: { state: done }
            "
        })
        .expect("agent config should parse");
        InferenceWorkerContext {
            _temp_dir: temp_dir,
            sessions_dir,
            session,
            agent_config,
        }
    }

    fn enable_codex_inference(agent_config: &mut AgentConfig, shadow_mode: bool) {
        agent_config.codex_status_inference = CodexStatusInferenceConfig {
            enabled: true,
            shadow_mode,
            failure_status: Some(StatusInferenceStatusConfig {
                kind: "needs_user".to_string(),
                note: "status inference failure investigation".to_string(),
            }),
            background_status: Some(StatusInferenceStatusConfig {
                kind: "external_wait".to_string(),
                note: "background task completion".to_string(),
            }),
        };
        agent_config
            .validate()
            .expect("Codex inference config should be valid");
    }

    fn codex_snapshot() -> CodexStopSnapshot {
        CodexStopSnapshot {
            thread_id: "fixture-thread".to_string(),
            turn_id: Some("fixture-turn".to_string()),
            last_assistant_message: Some("waiting for review".to_string()),
        }
    }

    fn args_for(session: &Session) -> StatusInferenceArgs {
        StatusInferenceArgs {
            session: session.session_id.clone(),
            updated_at: session.updated_at,
            codex_snapshot_file: None,
        }
    }

    fn stored_session(context: &InferenceWorkerContext) -> serde_json::Value {
        let session = store::load_session_from(&context.sessions_dir, &context.session.session_id)
            .expect("session should load")
            .expect("session should still exist");
        serde_json::to_value(session).expect("session should serialize")
    }

    fn session_value(session: &Session) -> serde_json::Value {
        serde_json::to_value(session).expect("session should serialize")
    }

    #[rstest]
    fn claude_inference_result_is_written_to_the_stopped_session(
        inference_worker_context: InferenceWorkerContext,
    ) {
        let context = inference_worker_context;
        let args = args_for(&context.session);
        let inferred_status = AgentStatus {
            kind: "external_wait".to_string(),
            note: "wait for the service".to_string(),
        };
        run_inner_with(
            &args,
            &context.sessions_dir,
            &context.agent_config,
            Ok(None),
            |_, _| Ok(inferred_status.clone()),
            |_, _, _, _| Err(anyhow!("Codex inference must not run")),
        )
        .expect("status inference should succeed");

        let mut expected = context.session.clone();
        expected.agent_status = Some(inferred_status);
        expected.agent_status_clear_pending = false;
        assert_eq!(stored_session(&context), session_value(&expected));
    }

    #[rstest]
    fn failed_claude_inference_leaves_the_session_unchanged(
        inference_worker_context: InferenceWorkerContext,
    ) {
        let context = inference_worker_context;
        let args = args_for(&context.session);
        let result = run_inner_with(
            &args,
            &context.sessions_dir,
            &context.agent_config,
            Ok(None),
            |_, _| Err(anyhow!("Claude fork failed")),
            |_, _, _, _| Err(anyhow!("Codex inference must not run")),
        );

        assert_eq!(
            (
                result.map_err(|error| format!("{error:#}")),
                stored_session(&context),
            ),
            (
                Err("Claude fork failed".to_string()),
                session_value(&context.session),
            ),
        );
    }

    #[rstest]
    fn disabled_codex_worker_skips_inference(inference_worker_context: InferenceWorkerContext) {
        let mut context = inference_worker_context;
        context.session.engine = Engine::Codex;
        store::save_session_to(&context.sessions_dir, &context.session)
            .expect("Codex session should be saved");
        let args = args_for(&context.session);
        let mut calls = Vec::new();
        run_inner_with(
            &args,
            &context.sessions_dir,
            &context.agent_config,
            Ok(Some(codex_snapshot())),
            |_, _| Err(anyhow!("Claude inference must not run")),
            |_, _, _, _| {
                calls.push("called".to_string());
                Err(anyhow!("Codex inference must not run"))
            },
        )
        .expect("disabled Codex inference should be skipped");

        assert_eq!(
            (calls, stored_session(&context)),
            (Vec::<String>::new(), session_value(&context.session))
        );
    }

    #[rstest]
    fn codex_inference_receives_the_stop_snapshot_and_writes_status(
        inference_worker_context: InferenceWorkerContext,
    ) {
        let mut context = inference_worker_context;
        context.session.engine = Engine::Codex;
        store::save_session_to(&context.sessions_dir, &context.session)
            .expect("Codex session should be saved");
        enable_codex_inference(&mut context.agent_config, false);
        let args = args_for(&context.session);
        let inferred_status = AgentStatus {
            kind: "external_wait".to_string(),
            note: "wait for review".to_string(),
        };
        let invocation = Arc::new(Mutex::new(None));
        let captured = invocation.clone();
        run_inner_with(
            &args,
            &context.sessions_dir,
            &context.agent_config,
            Ok(Some(codex_snapshot())),
            |_, _| Err(anyhow!("Claude inference must not run")),
            move |session, snapshot, prompt, kinds| {
                *captured.lock().expect("invocation lock") = Some((
                    session.session_id.clone(),
                    snapshot.clone(),
                    prompt.to_string(),
                    kinds.to_vec(),
                ));
                Ok(inferred_status)
            },
        )
        .expect("Codex inference should succeed");

        let mut expected_session = context.session.clone();
        expected_session.agent_status = Some(AgentStatus {
            kind: "external_wait".to_string(),
            note: "wait for review".to_string(),
        });
        let expected_invocation = Some((
            "fixture-session".to_string(),
            codex_snapshot(),
            build_codex_prompt(
                "Classify why the session stopped.",
                &context.session,
                &context.agent_config,
            ),
            vec![
                "external_wait".to_string(),
                "finished".to_string(),
                "needs_user".to_string(),
            ],
        ));
        assert_eq!(
            (
                stored_session(&context),
                invocation.lock().expect("invocation lock").clone()
            ),
            (session_value(&expected_session), expected_invocation),
        );
    }

    #[derive(Clone, Copy)]
    enum CodexInferenceFailure {
        MissingSnapshot,
        SnapshotRead,
        HistoryRead,
        Classification,
        InvalidJson,
        Timeout,
    }

    #[rstest]
    #[case::missing_snapshot(CodexInferenceFailure::MissingSnapshot, false)]
    #[case::snapshot_read(CodexInferenceFailure::SnapshotRead, false)]
    #[case::history_read(CodexInferenceFailure::HistoryRead, true)]
    #[case::classification(CodexInferenceFailure::Classification, true)]
    #[case::invalid_json(CodexInferenceFailure::InvalidJson, true)]
    #[case::timeout(CodexInferenceFailure::Timeout, true)]
    fn codex_inference_failures_record_the_configured_fallback(
        #[case] failure: CodexInferenceFailure,
        #[case] expects_classifier_call: bool,
        inference_worker_context: InferenceWorkerContext,
    ) {
        let mut context = inference_worker_context;
        context.session.engine = Engine::Codex;
        store::save_session_to(&context.sessions_dir, &context.session)
            .expect("Codex session should be saved");
        enable_codex_inference(&mut context.agent_config, false);
        let args = args_for(&context.session);
        let (snapshot_result, backend_error) = match failure {
            CodexInferenceFailure::MissingSnapshot => (Ok(None), None),
            CodexInferenceFailure::SnapshotRead => {
                (Err(anyhow!("snapshot acquisition failed")), None)
            }
            CodexInferenceFailure::HistoryRead => (
                Ok(Some(codex_snapshot())),
                Some("Codex app-server history request timed out"),
            ),
            CodexInferenceFailure::Classification => (
                Ok(Some(codex_snapshot())),
                Some("Claude standalone classifier process failed"),
            ),
            CodexInferenceFailure::InvalidJson => (
                Ok(Some(codex_snapshot())),
                Some("Claude returned malformed JSON"),
            ),
            CodexInferenceFailure::Timeout => (
                Ok(Some(codex_snapshot())),
                Some("Claude standalone classifier timed out"),
            ),
        };
        let mut classifier_called = false;
        run_inner_with(
            &args,
            &context.sessions_dir,
            &context.agent_config,
            snapshot_result,
            |_, _| Err(anyhow!("Claude inference must not run")),
            |_, _, _, _| {
                classifier_called = true;
                Err(anyhow!(
                    backend_error.unwrap_or("Codex classifier must not run")
                ))
            },
        )
        .expect("Codex inference failure should be handled");

        let mut expected = context.session.clone();
        expected.agent_status = Some(AgentStatus {
            kind: "needs_user".to_string(),
            note: "status inference failure investigation".to_string(),
        });
        assert_eq!(
            (stored_session(&context), classifier_called),
            (session_value(&expected), expects_classifier_call),
        );
    }

    #[rstest]
    fn codex_shadow_mode_runs_classifier_without_writing_status(
        inference_worker_context: InferenceWorkerContext,
    ) {
        let mut context = inference_worker_context;
        context.session.engine = Engine::Codex;
        store::save_session_to(&context.sessions_dir, &context.session)
            .expect("Codex session should be saved");
        enable_codex_inference(&mut context.agent_config, true);
        let args = args_for(&context.session);
        let mut classifier_called = false;
        let (result, log_events) = capture_info_json_events(|| {
            run_inner_with(
                &args,
                &context.sessions_dir,
                &context.agent_config,
                Ok(Some(codex_snapshot())),
                |_, _| Err(anyhow!("Claude inference must not run")),
                |_, _, _, _| {
                    classifier_called = true;
                    Ok(AgentStatus {
                        kind: "finished".to_string(),
                        note: "work is complete".to_string(),
                    })
                },
            )
        });
        result.expect("shadow inference should succeed");

        assert_eq!(
            (stored_session(&context), classifier_called, log_events),
            (
                session_value(&context.session),
                true,
                vec![serde_json::json!({
                    "event": "agent.status_inference.shadow_result",
                    "session": "fixture-session",
                    "kind": "finished",
                    "note": "work is complete",
                })],
            ),
        );
    }

    #[rstest]
    fn codex_shadow_mode_logs_failure_candidate_without_writing_status(
        inference_worker_context: InferenceWorkerContext,
    ) {
        let mut context = inference_worker_context;
        context.session.engine = Engine::Codex;
        store::save_session_to(&context.sessions_dir, &context.session)
            .expect("Codex session should be saved");
        enable_codex_inference(&mut context.agent_config, true);
        let args = args_for(&context.session);
        let (result, log_events) = capture_info_json_events(|| {
            run_inner_with(
                &args,
                &context.sessions_dir,
                &context.agent_config,
                Err(anyhow!("snapshot file is unavailable")),
                |_, _| Err(anyhow!("Claude inference must not run")),
                |_, _, _, _| Err(anyhow!("Codex inference must not run")),
            )
        });
        result.expect("shadow failure should be handled");

        assert_eq!(
            (stored_session(&context), log_events),
            (
                session_value(&context.session),
                vec![serde_json::json!({
                    "event": "agent.status_inference.shadow_failure_candidate",
                    "session": "fixture-session",
                    "kind": "needs_user",
                    "note": "status inference failure investigation",
                    "error": "snapshot file is unavailable",
                }),],
            ),
        );
    }

    #[rstest]
    fn codex_shadow_mode_discards_failure_candidate_after_manual_status(
        inference_worker_context: InferenceWorkerContext,
    ) {
        let mut context = inference_worker_context;
        context.session.engine = Engine::Codex;
        store::save_session_to(&context.sessions_dir, &context.session)
            .expect("Codex session should be saved");
        enable_codex_inference(&mut context.agent_config, true);
        let args = args_for(&context.session);
        let mut expected = context.session.clone();
        expected.agent_status = Some(AgentStatus {
            kind: "finished".to_string(),
            note: "manual status takes precedence".to_string(),
        });
        let (result, log_events) = capture_info_json_events(|| {
            run_inner_with(
                &args,
                &context.sessions_dir,
                &context.agent_config,
                Ok(Some(codex_snapshot())),
                |_, _| Err(anyhow!("Claude inference must not run")),
                |_, _, _, _| {
                    store::save_session_to(&context.sessions_dir, &expected)
                        .expect("manual status should be saved");
                    Err(anyhow!("Codex classifier failed"))
                },
            )
        });
        result.expect("shadow failure should be handled");

        assert_eq!(
            (stored_session(&context), log_events),
            (session_value(&expected), Vec::new()),
        );
    }

    #[rstest]
    fn codex_result_is_discarded_after_a_new_turn_starts(
        inference_worker_context: InferenceWorkerContext,
    ) {
        let mut context = inference_worker_context;
        context.session.engine = Engine::Codex;
        store::save_session_to(&context.sessions_dir, &context.session)
            .expect("Codex session should be saved");
        enable_codex_inference(&mut context.agent_config, false);
        let args = args_for(&context.session);
        let mut expected = context.session.clone();
        expected.updated_at = Utc::now() + chrono::Duration::seconds(1);
        run_inner_with(
            &args,
            &context.sessions_dir,
            &context.agent_config,
            Ok(Some(codex_snapshot())),
            |_, _| Err(anyhow!("Claude inference must not run")),
            |_, _, _, _| {
                store::save_session_to(&context.sessions_dir, &expected)?;
                Ok(AgentStatus {
                    kind: "finished".to_string(),
                    note: "work is complete".to_string(),
                })
            },
        )
        .expect("stale result should be discarded");

        assert_eq!(stored_session(&context), session_value(&expected));
    }

    #[rstest]
    fn codex_result_is_discarded_after_an_explicit_status_is_set(
        inference_worker_context: InferenceWorkerContext,
    ) {
        let mut context = inference_worker_context;
        context.session.engine = Engine::Codex;
        store::save_session_to(&context.sessions_dir, &context.session)
            .expect("Codex session should be saved");
        enable_codex_inference(&mut context.agent_config, false);
        let args = args_for(&context.session);
        let mut expected = context.session.clone();
        expected.updated_at = Utc::now() + chrono::Duration::seconds(1);
        expected.agent_status = Some(AgentStatus {
            kind: "needs_user".to_string(),
            note: "review this result".to_string(),
        });
        run_inner_with(
            &args,
            &context.sessions_dir,
            &context.agent_config,
            Ok(Some(codex_snapshot())),
            |_, _| Err(anyhow!("Claude inference must not run")),
            |_, _, _, _| {
                store::save_session_to(&context.sessions_dir, &expected)?;
                Ok(AgentStatus {
                    kind: "finished".to_string(),
                    note: "work is complete".to_string(),
                })
            },
        )
        .expect("changed session should skip inference");

        assert_eq!(stored_session(&context), session_value(&expected));
    }

    #[rstest]
    fn detached_worker_receives_the_complete_codex_stop_snapshot(
        inference_worker_context: InferenceWorkerContext,
    ) {
        let context = inference_worker_context;
        let request = StatusInferenceSpawnRequest {
            session_id: context.session.session_id,
            stop_updated_at: context.session.updated_at,
            codex_snapshot: Some(codex_snapshot()),
        };
        let executable = context._temp_dir.path().join("armyknife-fixture");
        let invocation = Arc::new(Mutex::new(None));
        let captured = invocation.clone();
        spawn_in_background_with(
            &request,
            &executable,
            context._temp_dir.path(),
            move |program, args| {
                let path = args
                    .windows(2)
                    .find(|pair| pair[0] == "--codex-snapshot-file")
                    .map(|pair| PathBuf::from(&pair[1]))
                    .ok_or_else(|| std::io::Error::other("snapshot path missing"))?;
                let snapshot: CodexStopSnapshot =
                    serde_json::from_slice(&std::fs::read(path).map_err(std::io::Error::other)?)
                        .map_err(std::io::Error::other)?;
                *captured.lock().expect("invocation lock") =
                    Some((program.to_path_buf(), args.to_vec(), snapshot));
                Ok(())
            },
        )
        .expect("detached worker should spawn");

        let actual = invocation.lock().expect("invocation lock").clone();
        let actual_args = actual.map(|(program, mut args, snapshot)| {
            let snapshot_path_index = args
                .windows(2)
                .position(|pair| pair[0] == "--codex-snapshot-file")
                .map(|index| index + 1);
            if let Some(index) = snapshot_path_index {
                args[index] = OsString::from("<snapshot-path>");
            }
            (
                program,
                args.iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
                snapshot,
            )
        });
        assert_eq!(
            actual_args,
            Some((
                executable,
                vec![
                    "agent".to_string(),
                    "status-inference".to_string(),
                    "--session".to_string(),
                    "fixture-session".to_string(),
                    "--updated-at".to_string(),
                    request.stop_updated_at.to_rfc3339(),
                    "--codex-snapshot-file".to_string(),
                    "<snapshot-path>".to_string(),
                ],
                codex_snapshot(),
            )),
        );
    }

    #[rstest]
    fn detached_worker_spawn_failure_removes_its_snapshot_file(
        inference_worker_context: InferenceWorkerContext,
    ) {
        let context = inference_worker_context;
        let request = StatusInferenceSpawnRequest {
            session_id: context.session.session_id,
            stop_updated_at: context.session.updated_at,
            codex_snapshot: Some(codex_snapshot()),
        };
        let captured_path = Arc::new(Mutex::new(None));
        let captured = captured_path.clone();
        let result = spawn_in_background_with(
            &request,
            Path::new("fixture-executable"),
            context._temp_dir.path(),
            move |_, args| {
                let path = args
                    .windows(2)
                    .find(|pair| pair[0] == "--codex-snapshot-file")
                    .map(|pair| PathBuf::from(&pair[1]));
                *captured.lock().expect("snapshot path lock") = path;
                Err(std::io::Error::other("fixture spawn failure"))
            },
        );
        let path = captured_path.lock().expect("snapshot path lock").clone();

        assert_eq!(
            (
                result.map_err(|error| format!("{error:#}")),
                path.as_ref().is_some_and(|path| path.exists()),
            ),
            (
                Err("failed to spawn status inference worker: fixture spawn failure".to_string()),
                false,
            ),
        );
    }
}
