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
    let usage_value = response.get("usage").cloned().unwrap_or_default();
    let usage: ClaudeUsage = serde_json::from_value(usage_value).context("parse Claude usage")?;
    Ok((parse_status_response(result)?, usage))
}

pub(super) fn parse_status_response(text: &str) -> Result<AgentStatus> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Response {
        kind: String,
        note: String,
    }

    let response: Response = serde_json::from_str(text).context("parse inferred status JSON")?;
    Ok(AgentStatus {
        kind: response.kind,
        note: response.note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::valid(r#"{"kind":"needs_user","note":"review the proposal"}"#, Ok(("needs_user".to_string(), "review the proposal".to_string())))]
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
