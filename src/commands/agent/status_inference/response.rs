use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;

use super::super::types::AgentStatus;

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub(super) struct ClaudeUsage {
    pub(super) input_tokens: Option<u64>,
    pub(super) cache_read_input_tokens: Option<u64>,
    pub(super) cache_creation_input_tokens: Option<u64>,
    pub(super) output_tokens: Option<u64>,
}

pub(super) fn parse_claude_output(stdout: &[u8]) -> Result<(AgentStatus, ClaudeUsage)> {
    let response: Value = serde_json::from_slice(stdout).context("parse Claude JSON response")?;
    ensure!(
        response.get("is_error").and_then(Value::as_bool) != Some(true),
        "Claude returned an error response"
    );
    let result = response
        .get("result")
        .and_then(Value::as_str)
        .context("Claude response has no result text")?;
    let usage = match response.get("usage") {
        Some(usage) => serde_json::from_value(usage.clone()).context("parse Claude usage")?,
        None => ClaudeUsage::default(),
    };
    Ok((parse_status_response(result)?, usage))
}

pub(in crate::commands::agent) fn parse_claude_status(stdout: &[u8]) -> Result<AgentStatus> {
    parse_claude_output(stdout).map(|(status, _)| status)
}

pub(super) fn parse_status_response(text: &str) -> Result<AgentStatus> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Response {
        kind: String,
        note: String,
    }

    let text = text.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|text| text.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(text);
    let response: Response = serde_json::from_str(text).context("parse inferred status JSON")?;
    Ok(AgentStatus {
        kind: response.kind,
        note: response.note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;
    use serde_json::json;

    #[test]
    fn claude_output_without_usage_uses_empty_usage() {
        let actual = parse_claude_output(
            &serde_json::to_vec(&json!({
                "result": r#"{"kind":"ready","note":"waiting"}"#,
            }))
            .expect("response should serialize"),
        )
        .map_err(|error| format!("{error:#}"));

        assert_eq!(
            actual,
            Ok((
                AgentStatus {
                    kind: "ready".to_string(),
                    note: "waiting".to_string(),
                },
                ClaudeUsage::default(),
            )),
        );
    }

    #[rstest]
    #[case::valid(r#"{"kind":"needs_user","note":"review the proposal"}"#, Ok(("needs_user".to_string(), "review the proposal".to_string())))]
    #[case::fenced_json(
        indoc! {r#"
            ```json
            {"kind":"needs_user","note":"review the proposal"}
            ```"#},
        Ok(("needs_user".to_string(), "review the proposal".to_string())),
    )]
    #[case::unknown_kind(r#"{"kind":"unknown","note":"review the proposal"}"#, Ok(("unknown".to_string(), "review the proposal".to_string())))]
    #[case::multiline_note(r#"{"kind":"needs_user","note":"first line\nsecond line"}"#, Ok(("needs_user".to_string(), "first line\nsecond line".to_string())))]
    #[case::extra_field(r#"{"kind":"needs_user","note":"review the proposal","extra":true}"#, Err("parse inferred status JSON: unknown field `extra`, expected `kind` or `note` at line 1 column 57".to_string()))]
    fn status_response_is_deserialized(
        #[case] text: &str,
        #[case] expected: std::result::Result<(String, String), String>,
    ) {
        let actual = parse_status_response(text)
            .map(|status| (status.kind, status.note))
            .map_err(|error| format!("{error:#}"));
        assert_eq!(actual, expected);
    }
}
