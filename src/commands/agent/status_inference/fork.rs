use std::ffi::OsString;
use std::fs;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use super::super::types::{AgentStatus, Session};
use super::response::parse_claude_output;
use crate::infra::external_tool::ExternalTool;
use crate::infra::process;

const INFERENCE_TIMEOUT: Duration = Duration::from_secs(120);
const FORK_SETTINGS: &str = r#"{"disableAllHooks":true,"permissions":{"ask":["*"]}}"#;

pub(super) fn infer_claude_status(session: &Session, prompt: &str) -> Result<AgentStatus> {
    let transcript_path = session
        .transcript_path
        .as_deref()
        .context("Claude session has no transcript path")?;
    let model = latest_model(transcript_path)?;
    let mut command = ExternalTool::Claude.command();
    command
        .args(claude_args(&session.session_id, &model, prompt))
        .current_dir(&session.cwd)
        .env("ARMYKNIFE_SKIP_HOOKS", "1");

    let output = process::run_with_timeout(command, INFERENCE_TIMEOUT)
        .context("Claude status inference process failed")?;
    ensure!(
        output.status.success(),
        "Claude status inference exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );

    let (status, usage) = parse_claude_output(&output.stdout)?;
    tracing::info!(
        event = "agent.status_inference.claude_usage",
        session = %session.session_id,
        input_tokens = usage.input_tokens,
        cache_read_input_tokens = usage.cache_read_input_tokens,
        cache_creation_input_tokens = usage.cache_creation_input_tokens,
        output_tokens = usage.output_tokens,
    );
    Ok(status)
}

fn claude_args(session_id: &str, model: &str, prompt: &str) -> Vec<OsString> {
    [
        "--resume",
        session_id,
        "--fork-session",
        "--no-session-persistence",
        "--settings",
        FORK_SETTINGS,
        "--permission-mode",
        "dontAsk",
        "--permission-prompts",
        "none",
        "--output-format",
        "json",
        "--model",
        model,
        "-p",
        prompt,
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

fn latest_model(transcript_path: &Path) -> Result<String> {
    let file = fs::File::open(transcript_path)
        .with_context(|| format!("open Claude transcript {}", transcript_path.display()))?;
    let mut model = None;
    for line in BufReader::new(file).lines() {
        let line = line?;
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if let Some(value) = event
            .pointer("/message/model")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            model = Some(value.to_string());
        }
    }
    model.context("Claude transcript contains no model")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn claude_arguments_keep_tool_definitions_and_block_execution() {
        assert_eq!(
            claude_args("fixture-session", "fixture-model", "infer the status"),
            [
                "--resume",
                "fixture-session",
                "--fork-session",
                "--no-session-persistence",
                "--settings",
                FORK_SETTINGS,
                "--permission-mode",
                "dontAsk",
                "--permission-prompts",
                "none",
                "--output-format",
                "json",
                "--model",
                "fixture-model",
                "-p",
                "infer the status",
            ]
            .map(OsString::from),
        );
    }

    #[test]
    fn latest_model_uses_the_last_assistant_model_record() {
        let mut transcript = tempfile::NamedTempFile::new().expect("transcript should be created");
        writeln!(transcript, "{{\"type\":\"user\",\"message\":{{}}}}")
            .expect("user event should be written");
        writeln!(
            transcript,
            "{{\"type\":\"assistant\",\"message\":{{\"model\":\"fixture-model-old\"}}}}"
        )
        .expect("first assistant event should be written");
        writeln!(
            transcript,
            "{{\"type\":\"assistant\",\"message\":{{\"model\":\"fixture-model-latest\"}}}}"
        )
        .expect("latest assistant event should be written");

        assert_eq!(
            latest_model(transcript.path()).expect("model should be found"),
            "fixture-model-latest",
        );
    }
}
