use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TokenUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
}

impl TokenUsage {
    pub(super) fn log(self, session_id: &str) {
        tracing::info!(
            event = "agent.status_inference.usage",
            session = %session_id,
            input_tokens = self.input_tokens,
            cached_input_tokens = self.cached_input_tokens,
            output_tokens = self.output_tokens,
        );
    }
}

pub(super) fn from_http_body(body: &[u8]) -> Option<TokenUsage> {
    if let Ok(response) = serde_json::from_slice(body)
        && let Some(usage) = from_response_value(&response)
    {
        return Some(usage);
    }

    body.split(|byte| *byte == b'\n')
        .filter_map(parse_sse_data_line)
        .filter_map(|event| from_response_value(&event))
        .next_back()
}

pub(super) fn from_response_value(response: &Value) -> Option<TokenUsage> {
    let usage = response
        .get("response")
        .and_then(|response| response.get("usage"))
        .or_else(|| response.get("usage"))?;
    let input_tokens = usage.get("input_tokens")?.as_u64()?;
    let cached_input_tokens = usage
        .get("input_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let output_tokens = usage.get("output_tokens")?.as_u64()?;
    Some(TokenUsage {
        input_tokens,
        cached_input_tokens,
        output_tokens,
    })
}

fn parse_sse_data_line(line: &[u8]) -> Option<Value> {
    let line = super::trim_ascii(line).strip_prefix(b"data:")?;
    let data = super::trim_ascii(line);
    if data == b"[DONE]" {
        return None;
    }
    serde_json::from_slice(data).ok()
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::rstest;
    use serde_json::{Value, json};

    use super::{TokenUsage, from_http_body, from_response_value};

    #[rstest]
    #[case::completed_response(
        json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 321,
                    "input_tokens_details": { "cached_tokens": 123 },
                    "output_tokens": 45,
                },
            },
        }),
        Some(TokenUsage {
            input_tokens: 321,
            cached_input_tokens: 123,
            output_tokens: 45,
        }),
    )]
    #[case::missing_cached_count(
        json!({
            "usage": {
                "input_tokens": 7,
                "output_tokens": 3,
            },
        }),
        Some(TokenUsage {
            input_tokens: 7,
            cached_input_tokens: 0,
            output_tokens: 3,
        }),
    )]
    #[case::malformed_usage(json!({ "usage": { "input_tokens": "hidden" } }), None)]
    fn extracts_only_numeric_usage(#[case] response: Value, #[case] expected: Option<TokenUsage>) {
        assert_eq!(from_response_value(&response), expected);
    }

    #[test]
    fn extracts_final_usage_from_streamed_http_events() {
        let body = indoc! {r#"
            event: response.output_text.delta
            data: {"type":"response.output_text.delta","delta":"fixture text"}

            event: response.completed
            data: {"type":"response.completed","response":{"usage":{"input_tokens":80,"input_tokens_details":{"cached_tokens":40},"output_tokens":12}}}

            data: [DONE]
        "#};

        assert_eq!(
            from_http_body(body.as_bytes()),
            Some(TokenUsage {
                input_tokens: 80,
                cached_input_tokens: 40,
                output_tokens: 12,
            }),
        );
    }
}
