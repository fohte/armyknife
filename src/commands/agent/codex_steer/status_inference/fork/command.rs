use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, ensure};
use serde_json::to_string;

use super::super::super::history::CodexThreadMetadata;
use super::path_to_string;
use crate::commands::agent::types::Session;
use crate::infra::external_tool::ExternalTool;

pub(super) fn codex_fork_command(
    session: &Session,
    thread: &CodexThreadMetadata,
    prompt: &str,
    result_path: &Path,
    settings_path: &Path,
    base_url: &str,
    inference_dir: &Path,
) -> Result<Command> {
    let codex_path = ExternalTool::Codex
        .resolve_path()
        .context("Codex CLI is not available on PATH")?;
    let mut command = ExternalTool::SandboxRuntime.command();
    command
        .arg("--settings")
        .arg(settings_path)
        .arg(codex_path)
        .args(codex_fork_args(
            &thread.session_id,
            &thread.model,
            prompt,
            result_path,
            base_url,
        )?)
        .current_dir(&session.cwd)
        .env("ARMYKNIFE_SKIP_HOOKS", "1")
        .env("CLAUDE_CODE_TMPDIR", inference_dir)
        .env("CLAUDE_TMPDIR", inference_dir)
        .env_remove("OPENAI_API_KEY")
        .env_remove("CODEX_API_KEY")
        .env_remove("CODEX_ACCESS_TOKEN");
    Ok(command)
}

fn codex_fork_args(
    session_id: &str,
    model: &str,
    prompt: &str,
    result_path: &Path,
    base_url: &str,
) -> Result<Vec<String>> {
    ensure!(!session_id.is_empty(), "Codex parent session ID is empty");
    ensure!(!model.is_empty(), "Codex parent model is empty");
    Ok(vec![
        "exec".to_string(),
        "fork".to_string(),
        "--ephemeral".to_string(),
        "--model".to_string(),
        model.to_string(),
        "--config".to_string(),
        "model_provider=\"openai\"".to_string(),
        "--config".to_string(),
        format!("openai_base_url={}", to_string(base_url)?),
        "--config".to_string(),
        "features.hooks=false".to_string(),
        "--config".to_string(),
        "features.enable_request_compression=false".to_string(),
        "--output-last-message".to_string(),
        path_to_string(result_path)?,
        "--".to_string(),
        session_id.to_string(),
        prompt.to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::codex_fork_args;

    #[test]
    fn codex_fork_uses_parent_model_ephemeral_history_and_relay_overrides() {
        let actual = codex_fork_args(
            "fixture-parent-session",
            "fixture-model",
            "classify this stopped task",
            Path::new("/private/tmp/status-worker-fixture/result.json"),
            "http://localhost:43817/fixture-token/backend-api/codex",
        );
        let actual = actual.expect("fixture fork arguments should be valid");
        assert_eq!(
            actual,
            vec![
                "exec",
                "fork",
                "--ephemeral",
                "--model",
                "fixture-model",
                "--config",
                "model_provider=\"openai\"",
                "--config",
                "openai_base_url=\"http://localhost:43817/fixture-token/backend-api/codex\"",
                "--config",
                "features.hooks=false",
                "--config",
                "features.enable_request_compression=false",
                "--output-last-message",
                "/private/tmp/status-worker-fixture/result.json",
                "--",
                "fixture-parent-session",
                "classify this stopped task",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        );
    }
}
