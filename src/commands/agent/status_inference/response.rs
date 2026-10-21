use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

use super::super::types::AgentStatus;
use crate::shared::config::AgentConfig;

pub(super) fn status_schema(agent_config: &AgentConfig) -> Value {
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
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ClaudeUsage {
    pub(super) input_tokens: Option<u64>,
    pub(super) cache_read_input_tokens: Option<u64>,
    pub(super) cache_creation_input_tokens: Option<u64>,
    pub(super) output_tokens: Option<u64>,
}

pub(super) fn parse_claude_output(
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

pub(super) fn parse_status_response(text: &str, agent_config: &AgentConfig) -> Result<AgentStatus> {
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
pub(super) struct CodexUsage {
    pub(super) input_tokens: Option<u64>,
    pub(super) cached_input_tokens: Option<u64>,
    pub(super) output_tokens: Option<u64>,
}

pub(super) fn parse_codex_events(stdout: &[u8]) -> Result<CodexUsage> {
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
    use serde_json::json;

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
