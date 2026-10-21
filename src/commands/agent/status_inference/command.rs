use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Args;
use std::path::Path;

use super::super::types::{AgentStatus, Engine, Session};
use super::super::{status, store};
use super::fork::{infer_claude_status, infer_codex_status};
use super::prompt::build_prompt;
#[cfg(not(test))]
use crate::infra::process;
use crate::shared::config::{self, AgentConfig};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct StatusInferenceArgs {
    /// Session whose Stop event requested status inference.
    #[arg(long)]
    pub session: String,

    /// Session-record timestamp captured by that Stop event.
    #[arg(long, value_parser = parse_updated_at)]
    pub updated_at: DateTime<Utc>,
}

fn parse_updated_at(value: &str) -> std::result::Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|date_time| date_time.with_timezone(&Utc))
        .map_err(|error| format!("invalid RFC 3339 timestamp: {error}"))
}

#[cfg(not(test))]
pub fn spawn_in_background(session_id: &str, stop_updated_at: DateTime<Utc>) {
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(
                event = "agent.status_inference.spawn_failed",
                session = %session_id,
                error = %error,
            );
            return;
        }
    };
    let updated_at = stop_updated_at.to_rfc3339();
    let args = [
        "agent",
        "status-inference",
        "--session",
        session_id,
        "--updated-at",
        updated_at.as_str(),
    ];
    match process::spawn_detached(executable, args, None, &[("ARMYKNIFE_SKIP_HOOKS", "1")]) {
        Ok(()) => tracing::info!(
            event = "agent.status_inference.spawned",
            session = %session_id,
        ),
        Err(error) => tracing::warn!(
            event = "agent.status_inference.spawn_failed",
            session = %session_id,
            error = %format!("{error:#}"),
        ),
    }
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
    let Some(configured_prompt) = agent_config.configured_status_inference_prompt() else {
        let reason = if agent_config.statuses.is_empty() {
            "statuses_not_configured"
        } else {
            "prompt_not_configured"
        };
        tracing::info!(
            event = "agent.status_inference.skipped",
            session = %args.session,
            reason,
        );
        return Ok(());
    };

    let sessions_dir = store::sessions_dir()?;
    run_inner_with(
        args,
        &sessions_dir,
        &agent_config,
        configured_prompt,
        infer_claude_status,
        infer_codex_status,
    )
}

fn run_inner_with<ClaudeInference, CodexInference>(
    args: &StatusInferenceArgs,
    sessions_dir: &Path,
    agent_config: &AgentConfig,
    configured_prompt: &str,
    infer_claude: ClaudeInference,
    infer_codex: CodexInference,
) -> Result<()>
where
    ClaudeInference: FnOnce(&Session, &str, &AgentConfig) -> Result<AgentStatus>,
    CodexInference: FnOnce(&Session, &str, &AgentConfig) -> Result<AgentStatus>,
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

    let prompt = build_prompt(configured_prompt, &session, agent_config);
    let inferred_status = match session.engine {
        Engine::Claude => infer_claude(&session, &prompt, agent_config)?,
        Engine::Codex => infer_codex(&session, &prompt, agent_config)?,
    };

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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use chrono::Utc;
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;
    use crate::commands::agent::types::SessionStatus;

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
            engine: Engine::Codex,
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

    #[rstest]
    fn codex_inference_result_is_written_to_the_stopped_session(
        inference_worker_context: InferenceWorkerContext,
    ) {
        let context = inference_worker_context;
        let args = StatusInferenceArgs {
            session: context.session.session_id.clone(),
            updated_at: context.session.updated_at,
        };
        let inferred_status = AgentStatus {
            kind: "external_wait".to_string(),
            note: "wait for the service".to_string(),
        };
        run_inner_with(
            &args,
            &context.sessions_dir,
            &context.agent_config,
            context
                .agent_config
                .configured_status_inference_prompt()
                .expect("status inference should be enabled"),
            |_, _, _| Err(anyhow::anyhow!("Claude runner must not be used")),
            |_, _, _| Ok(inferred_status.clone()),
        )
        .expect("status inference should succeed");

        let actual = store::load_session_from(&context.sessions_dir, &args.session)
            .expect("session should load")
            .expect("session should still exist");
        let mut expected = context.session;
        expected.agent_status = Some(inferred_status);
        expected.agent_status_clear_pending = false;
        assert_eq!(
            serde_json::to_value(actual).expect("session should serialize"),
            serde_json::to_value(expected).expect("expected session should serialize"),
        );
    }

    #[rstest]
    fn failed_codex_inference_leaves_the_session_unchanged(
        inference_worker_context: InferenceWorkerContext,
    ) {
        let context = inference_worker_context;
        let args = StatusInferenceArgs {
            session: context.session.session_id.clone(),
            updated_at: context.session.updated_at,
        };
        let result = run_inner_with(
            &args,
            &context.sessions_dir,
            &context.agent_config,
            context
                .agent_config
                .configured_status_inference_prompt()
                .expect("status inference should be enabled"),
            |_, _, _| Err(anyhow::anyhow!("Claude runner must not be used")),
            |_, _, _| Err(anyhow::anyhow!("Codex fork failed")),
        );
        let actual = store::load_session_from(&context.sessions_dir, &args.session)
            .expect("session should load")
            .expect("session should still exist");

        assert_eq!(
            (
                result.map_err(|error| format!("{error:#}")),
                serde_json::to_value(actual).expect("session should serialize"),
            ),
            (
                Err("Codex fork failed".to_string()),
                serde_json::to_value(context.session).expect("expected session should serialize"),
            ),
        );
    }
}
