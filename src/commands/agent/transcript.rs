use serde_json::Value;

pub(super) fn claude_assistant_model(record: &Value) -> Option<&str> {
    if record.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }

    record
        .pointer("/message/model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
}
