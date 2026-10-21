use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::Value;
use tempfile::NamedTempFile;

use super::super::types::{AgentStatus, Session};
use super::response::{
    parse_claude_output, parse_codex_events, parse_status_response, status_schema,
};
use crate::infra::external_tool::ExternalTool;
use crate::infra::process;
use crate::shared::config::AgentConfig;

const INFERENCE_TIMEOUT: Duration = Duration::from_secs(120);
const CODEX_DISABLED_FEATURES: &[&str] = &[
    "hooks",
    "shell_tool",
    "plugins",
    "apps",
    "browser_annotation_api",
    "browser_use",
    "browser_use_external",
    "browser_use_full_cdp_access",
    "computer_use",
    "image_generation",
    "in_app_browser",
    "in_app_local_automation",
    "code_mode",
    "multi_agent",
    "view_image",
    "unified_exec",
    "unified_exec_tty",
    "sleep_tool",
    "skill_search",
    "auth_elicitation",
    "tool_call_mcp_elicitation",
    "tool_suggest",
];
pub(super) fn infer_claude_status(
    session: &Session,
    prompt: &str,
    agent_config: &AgentConfig,
) -> Result<AgentStatus> {
    let transcript_path = session
        .transcript_path
        .as_deref()
        .context("Claude session has no transcript path")?;
    let model = latest_model(
        transcript_path,
        "Claude",
        Some("assistant"),
        &["/message/model"],
    )?;
    let schema = serde_json::to_string(&status_schema(agent_config))
        .context("serialize Claude output schema")?;
    let mut command = ExternalTool::Claude.command();
    command
        .args(claude_args(&session.session_id, &model, &schema, prompt))
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

pub(super) fn infer_codex_status(
    session: &Session,
    prompt: &str,
    agent_config: &AgentConfig,
) -> Result<AgentStatus> {
    let transcript_path = session
        .transcript_path
        .as_deref()
        .context("Codex session has no transcript path")?;
    let model = latest_model(
        transcript_path,
        "Codex",
        None,
        &["/payload/model", "/message/model"],
    )?;
    let schema_file = status_schema_file(agent_config, "Codex")?;
    let output_file = NamedTempFile::new().context("create Codex output file")?;

    let mut command = ExternalTool::Codex.command();
    command
        .args(codex_args(
            &session.session_id,
            &model,
            schema_file.path(),
            output_file.path(),
            prompt,
        ))
        .current_dir(&session.cwd)
        .env("ARMYKNIFE_SKIP_HOOKS", "1");

    let output = process::run_with_timeout(command, INFERENCE_TIMEOUT)
        .context("Codex status inference process failed")?;
    ensure!(
        output.status.success(),
        "Codex status inference exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );

    let usage = parse_codex_events(&output.stdout)?;
    tracing::info!(
        event = "agent.status_inference.codex_usage",
        session = %session.session_id,
        input_tokens = usage.input_tokens,
        cached_input_tokens = usage.cached_input_tokens,
        output_tokens = usage.output_tokens,
    );
    let response = fs::read_to_string(output_file.path()).context("read Codex final response")?;
    parse_status_response(&response)
}

fn status_schema_file(agent_config: &AgentConfig, engine: &str) -> Result<NamedTempFile> {
    let mut schema_file =
        NamedTempFile::new().with_context(|| format!("create {engine} output schema"))?;
    serde_json::to_writer(&mut schema_file, &status_schema(agent_config))?;
    schema_file.flush()?;
    Ok(schema_file)
}

fn claude_args(session_id: &str, model: &str, schema: &str, prompt: &str) -> Vec<OsString> {
    let mut args = [
        "--resume",
        session_id,
        "--fork-session",
        "--no-session-persistence",
        "--safe-mode",
        "--tools",
        "",
        "--disallowedTools",
        "mcp__*",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--strict-mcp-config",
        "--permission-prompts",
        "none",
        "--output-format",
        "json",
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    args.push("--json-schema".into());
    args.push(schema.into());
    args.extend(
        ["--model", model, "-p", prompt]
            .into_iter()
            .map(OsString::from),
    );
    args
}

fn codex_args(
    session_id: &str,
    model: &str,
    schema_path: &Path,
    output_path: &Path,
    prompt: &str,
) -> Vec<OsString> {
    let mut args = vec![
        "exec".into(),
        "fork".into(),
        session_id.into(),
        "--ephemeral".into(),
        "--json".into(),
        "--output-schema".into(),
        schema_path.as_os_str().to_os_string(),
        "-o".into(),
        output_path.as_os_str().to_os_string(),
        "--ignore-user-config".into(),
        "--config".into(),
        "mcp_servers={}".into(),
        "-m".into(),
        model.into(),
    ];
    for feature in CODEX_DISABLED_FEATURES {
        args.push("--disable".into());
        args.push((*feature).into());
    }
    args.push(prompt.into());
    args
}

fn latest_model(
    transcript_path: &Path,
    engine: &str,
    required_event_type: Option<&str>,
    model_pointers: &[&str],
) -> Result<String> {
    let file = fs::File::open(transcript_path)
        .with_context(|| format!("open {engine} transcript {}", transcript_path.display()))?;
    let mut model = None;
    for line in BufReader::new(file).lines() {
        let line = line?;
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(required_event_type) = required_event_type
            && event.get("type").and_then(Value::as_str) != Some(required_event_type)
        {
            continue;
        }
        if let Some(value) = model_pointers
            .iter()
            .find_map(|pointer| event.pointer(pointer).and_then(Value::as_str))
            .filter(|value| !value.is_empty())
        {
            model = Some(value.to_string());
        }
    }
    model.with_context(|| format!("{engine} transcript contains no model"))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_arguments_fork_the_original_session_without_tools() {
        assert_eq!(
            claude_args(
                "fixture-session",
                "fixture-model",
                r#"{"type":"object"}"#,
                "infer the status",
            ),
            [
                "--resume",
                "fixture-session",
                "--fork-session",
                "--no-session-persistence",
                "--safe-mode",
                "--tools",
                "",
                "--disallowedTools",
                "mcp__*",
                "--mcp-config",
                r#"{"mcpServers":{}}"#,
                "--strict-mcp-config",
                "--permission-prompts",
                "none",
                "--output-format",
                "json",
                "--json-schema",
                r#"{"type":"object"}"#,
                "--model",
                "fixture-model",
                "-p",
                "infer the status",
            ]
            .map(OsString::from),
        );
    }
    #[test]
    fn codex_arguments_disable_execution_and_session_hooks() {
        let actual = codex_args(
            "fixture-session",
            "fixture-codex-model",
            Path::new("schema.json"),
            Path::new("output.json"),
            "infer the status",
        );
        let expected: Vec<OsString> = vec![
            "exec".into(),
            "fork".into(),
            "fixture-session".into(),
            "--ephemeral".into(),
            "--json".into(),
            "--output-schema".into(),
            "schema.json".into(),
            "-o".into(),
            "output.json".into(),
            "--ignore-user-config".into(),
            "--config".into(),
            "mcp_servers={}".into(),
            "-m".into(),
            "fixture-codex-model".into(),
            "--disable".into(),
            "hooks".into(),
            "--disable".into(),
            "shell_tool".into(),
            "--disable".into(),
            "plugins".into(),
            "--disable".into(),
            "apps".into(),
            "--disable".into(),
            "browser_annotation_api".into(),
            "--disable".into(),
            "browser_use".into(),
            "--disable".into(),
            "browser_use_external".into(),
            "--disable".into(),
            "browser_use_full_cdp_access".into(),
            "--disable".into(),
            "computer_use".into(),
            "--disable".into(),
            "image_generation".into(),
            "--disable".into(),
            "in_app_browser".into(),
            "--disable".into(),
            "in_app_local_automation".into(),
            "--disable".into(),
            "code_mode".into(),
            "--disable".into(),
            "multi_agent".into(),
            "--disable".into(),
            "view_image".into(),
            "--disable".into(),
            "unified_exec".into(),
            "--disable".into(),
            "unified_exec_tty".into(),
            "--disable".into(),
            "sleep_tool".into(),
            "--disable".into(),
            "skill_search".into(),
            "--disable".into(),
            "auth_elicitation".into(),
            "--disable".into(),
            "tool_call_mcp_elicitation".into(),
            "--disable".into(),
            "tool_suggest".into(),
            "infer the status".into(),
        ];
        assert_eq!(actual, expected);
    }
    #[test]
    fn latest_model_uses_the_last_matching_model_record() {
        let mut transcript = NamedTempFile::new().expect("transcript file should be created");
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
            latest_model(
                transcript.path(),
                "Claude",
                Some("assistant"),
                &["/message/model"],
            )
            .expect("model should be found"),
            "fixture-model-latest",
        );
    }

    #[test]
    fn latest_model_uses_first_matching_pointer_for_codex_records() {
        let mut transcript = NamedTempFile::new().expect("transcript file should be created");
        writeln!(
            transcript,
            "{{\"type\":\"session_meta\",\"payload\":{{\"model\":\"fixture-codex-old\"}}}}"
        )
        .expect("session metadata should be written");
        writeln!(
            transcript,
            "{{\"type\":\"turn_context\",\"payload\":{{\"model\":\"fixture-codex-latest\"}}}}"
        )
        .expect("latest turn context should be written");

        assert_eq!(
            latest_model(
                transcript.path(),
                "Codex",
                None,
                &["/payload/model", "/message/model"],
            )
            .expect("model should be found"),
            "fixture-codex-latest",
        );
    }
}
