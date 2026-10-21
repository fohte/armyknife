use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use clap::Args;
use indoc::indoc;
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::NamedTempFile;

use super::status;
use super::store;
use super::types::{AgentStatus, Engine, Session};
use crate::infra::external_tool::ExternalTool;
use crate::infra::process;
use crate::shared::config::{self, AgentConfig, AgentStatusState};

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

fn build_prompt(configured_prompt: &str, session: &Session, agent_config: &AgentConfig) -> String {
    let mut kinds = agent_config
        .statuses
        .iter()
        .map(|(kind, config)| {
            let state = match config.state {
                AgentStatusState::User => "user",
                AgentStatusState::Waiting => "wait",
                AgentStatusState::Done => "done",
            };
            (kind.as_str(), state)
        })
        .collect::<Vec<_>>();
    kinds.sort_unstable_by_key(|(kind, _)| *kind);

    let statuses = kinds
        .into_iter()
        .map(|(kind, state)| format!("- {kind} (state: {state})"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        indoc! {"
            {}

            Review the full forked conversation and infer why the original agent stopped.
            Session information:
            - engine: {}
            - working directory: {}
            {}

            Choose exactly one configured status kind. Return only a JSON object with exactly two string fields: `kind` and `note`.
            The note must be a concise single line. Do not call tools.

            Configured status kinds:
            {}
        "},
        configured_prompt,
        session.engine.process_name(),
        session.cwd.display(),
        session
            .label
            .as_deref()
            .map(|label| format!("- session label: {label}"))
            .unwrap_or_default(),
        statuses,
    )
}

fn infer_claude_status(
    session: &Session,
    prompt: &str,
    agent_config: &AgentConfig,
) -> Result<AgentStatus> {
    let transcript_path = session
        .transcript_path
        .as_deref()
        .context("Claude session has no transcript path")?;
    let model = latest_assistant_model(transcript_path)?;
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

    let (status, usage) = parse_claude_output(&output.stdout, agent_config)?;
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

fn infer_codex_status(
    session: &Session,
    prompt: &str,
    agent_config: &AgentConfig,
) -> Result<AgentStatus> {
    let transcript_path = session
        .transcript_path
        .as_deref()
        .context("Codex session has no transcript path")?;
    let model = latest_codex_model(transcript_path)?;
    let mut schema_file = NamedTempFile::new().context("create Codex output schema")?;
    serde_json::to_writer(&mut schema_file, &status_schema(agent_config))?;
    schema_file.flush()?;
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
    parse_status_response(&response, agent_config)
}

fn claude_args(session_id: &str, model: &str, prompt: &str) -> Vec<OsString> {
    [
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
        "--model",
        model,
        "-p",
        prompt,
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
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

fn status_schema(agent_config: &AgentConfig) -> Value {
    let mut kinds = agent_config.statuses.keys().cloned().collect::<Vec<_>>();
    kinds.sort_unstable();
    json!({
        "type": "object",
        "properties": {
            "kind": { "type": "string", "enum": kinds },
            "note": {
                "type": "string",
                "minLength": 1,
                "pattern": "^[^\r\n]+$",
            },
        },
        "required": ["kind", "note"],
        "additionalProperties": false,
    })
}

fn latest_assistant_model(transcript_path: &Path) -> Result<String> {
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
    model.context("Claude transcript contains no assistant model")
}

fn latest_codex_model(transcript_path: &Path) -> Result<String> {
    let file = fs::File::open(transcript_path)
        .with_context(|| format!("open Codex transcript {}", transcript_path.display()))?;
    let mut model = None;
    for line in BufReader::new(file).lines() {
        let line = line?;
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(value) = event
            .pointer("/payload/model")
            .or_else(|| event.pointer("/message/model"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            model = Some(value.to_string());
        }
    }
    model.context("Codex transcript contains no model")
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ClaudeUsage {
    input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

fn parse_claude_output(
    stdout: &[u8],
    agent_config: &AgentConfig,
) -> Result<(AgentStatus, ClaudeUsage)> {
    let response: Value = serde_json::from_slice(stdout).context("parse Claude JSON response")?;
    ensure!(
        response.get("is_error").and_then(Value::as_bool) != Some(true),
        "Claude returned an error response"
    );
    let result = response
        .get("result")
        .and_then(Value::as_str)
        .context("Claude response has no result text")?;
    let usage_value = response.get("usage").cloned().unwrap_or_default();
    let fields: ClaudeUsageFields =
        serde_json::from_value(usage_value).context("parse Claude usage")?;
    let usage = ClaudeUsage {
        input_tokens: fields.input_tokens,
        cache_read_input_tokens: fields.cache_read_input_tokens,
        cache_creation_input_tokens: fields.cache_creation_input_tokens,
        output_tokens: fields.output_tokens,
    };
    Ok((parse_status_response(result, agent_config)?, usage))
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct ClaudeUsageFields {
    input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

fn parse_status_response(text: &str, agent_config: &AgentConfig) -> Result<AgentStatus> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Response {
        kind: String,
        note: String,
    }

    let response: Response = serde_json::from_str(text).context("parse inferred status JSON")?;
    ensure!(
        agent_config.statuses.contains_key(&response.kind),
        "inferred unknown status kind `{}`",
        response.kind
    );
    ensure!(
        !response.note.trim().is_empty(),
        "inferred status note is empty"
    );
    ensure!(
        !response.note.contains('\n') && !response.note.contains('\r'),
        "inferred status note must be a single line"
    );
    Ok(AgentStatus {
        kind: response.kind,
        note: response.note,
    })
}

#[derive(Debug, Default, PartialEq, Eq)]
struct CodexUsage {
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

fn parse_codex_events(stdout: &[u8]) -> Result<CodexUsage> {
    let stdout = std::str::from_utf8(stdout).context("Codex JSONL output is not UTF-8")?;
    let mut usage = CodexUsage::default();
    let mut saw_agent_message = false;
    let mut saw_completed_turn = false;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let event: Value = serde_json::from_str(line).context("parse Codex JSONL event")?;
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .context("Codex event has no type")?;
        if event_type.starts_with("item.") {
            let item_type = event
                .pointer("/item/type")
                .and_then(Value::as_str)
                .context("Codex item event has no item type")?;
            ensure!(
                item_type == "agent_message",
                "Codex fork attempted unsupported item type `{item_type}`"
            );
            saw_agent_message = true;
        }
        ensure!(
            event_type != "turn.failed" && event_type != "error",
            "Codex returned failure event `{event_type}`"
        );
        if event_type == "turn.completed" {
            saw_completed_turn = true;
            if let Some(value) = event.get("usage") {
                let fields: CodexUsageFields =
                    serde_json::from_value(value.clone()).context("parse Codex usage")?;
                usage = CodexUsage {
                    input_tokens: fields.input_tokens,
                    cached_input_tokens: fields.cached_input_tokens,
                    output_tokens: fields.output_tokens,
                };
            }
        }
    }
    ensure!(saw_agent_message, "Codex returned no agent message event");
    ensure!(saw_completed_turn, "Codex returned no completed turn event");
    Ok(usage)
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct CodexUsageFields {
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    fn agent_config() -> AgentConfig {
        serde_yaml::from_str(indoc! {
            "
            status_inference_prompt: Classify the stopping reason.
            statuses:
              needs_user: { state: user }
              external_wait: { state: waiting }
              finished: { state: done }
            "
        })
        .expect("agent config should parse")
    }

    #[rstest]
    #[case::valid(r#"{"kind":"needs_user","note":"review the proposal"}"#, Ok(("needs_user".to_string(), "review the proposal".to_string())))]
    #[case::unknown_kind(r#"{"kind":"unknown","note":"review the proposal"}"#, Err("inferred unknown status kind `unknown`".to_string()))]
    #[case::multiline_note(r#"{"kind":"needs_user","note":"first line\nsecond line"}"#, Err("inferred status note must be a single line".to_string()))]
    #[case::extra_field(r#"{"kind":"needs_user","note":"review the proposal","extra":true}"#, Err("parse inferred status JSON: unknown field `extra`, expected `kind` or `note` at line 1 column 57".to_string()))]
    fn status_response_is_validated(
        #[case] text: &str,
        #[case] expected: std::result::Result<(String, String), String>,
    ) {
        let actual = parse_status_response(text, &agent_config())
            .map(|status| (status.kind, status.note))
            .map_err(|error| format!("{error:#}"));
        assert_eq!(actual, expected);
    }

    #[test]
    fn claude_arguments_fork_the_original_session_without_tools() {
        assert_eq!(
            claude_args("fixture-session", "fixture-model", "infer the status"),
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
    fn latest_assistant_model_uses_the_last_assistant_record() {
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
            latest_assistant_model(transcript.path()).expect("model should be found"),
            "fixture-model-latest",
        );
    }

    #[test]
    fn latest_codex_model_uses_the_latest_turn_context() {
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
            latest_codex_model(transcript.path()).expect("model should be found"),
            "fixture-codex-latest",
        );
    }

    #[rstest]
    #[case::agent_message(
        indoc! {"\
            {\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"{}\"}}\n\
            {\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":12,\"cached_input_tokens\":8,\"output_tokens\":3}}\n"},
        Ok(CodexUsage { input_tokens: Some(12), cached_input_tokens: Some(8), output_tokens: Some(3) }),
    )]
    #[case::tool_attempt(
        indoc! {"\
            {\"type\":\"item.started\",\"item\":{\"type\":\"command_execution\"}}\n"},
        Err("Codex fork attempted unsupported item type `command_execution`".to_string()),
    )]
    fn codex_events_accept_only_agent_messages(
        #[case] stdout: &str,
        #[case] expected: std::result::Result<CodexUsage, String>,
    ) {
        let actual = parse_codex_events(stdout.as_bytes()).map_err(|error| error.to_string());
        assert_eq!(actual, expected);
    }

    #[test]
    fn schema_limits_kind_to_configured_statuses_and_note_to_one_line() {
        let schema = status_schema(&agent_config());
        assert_eq!(
            schema,
            json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["external_wait", "finished", "needs_user"] },
                    "note": { "type": "string", "minLength": 1, "pattern": "^[^\r\n]+$" },
                },
                "required": ["kind", "note"],
                "additionalProperties": false,
            }),
        );
    }
}
