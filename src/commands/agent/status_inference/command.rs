use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Args;

use super::super::{status, store, types::Engine};
use super::fork::{infer_claude_status, infer_codex_status};
use super::prompt::build_prompt;
#[cfg(not(test))]
use crate::infra::process;
use crate::shared::config;

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
    let Some(configured_prompt) = agent_config
        .status_inference_prompt
        .as_deref()
        .filter(|prompt| !prompt.trim().is_empty())
    else {
        tracing::info!(
            event = "agent.status_inference.skipped",
            session = %args.session,
            reason = "prompt_not_configured",
        );
        return Ok(());
    };
    if agent_config.statuses.is_empty() {
        tracing::info!(
            event = "agent.status_inference.skipped",
            session = %args.session,
            reason = "statuses_not_configured",
        );
        return Ok(());
    }

    let sessions_dir = store::sessions_dir()?;
    let Some(session) = store::load_session_from(&sessions_dir, &args.session)? else {
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

    let prompt = build_prompt(configured_prompt, &session, &agent_config);
    let inferred_status = match session.engine {
        Engine::Claude => infer_claude_status(&session, &prompt, &agent_config)?,
        Engine::Codex => infer_codex_status(&session, &prompt, &agent_config)?,
    };

    match status::set_inferred_status_if_unchanged(
        &sessions_dir,
        &args.session,
        args.updated_at,
        &agent_config,
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
