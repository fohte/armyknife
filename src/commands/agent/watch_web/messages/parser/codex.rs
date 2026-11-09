use std::io::BufRead;

use serde_json::Value;

use super::super::ChatItem;
use super::super::parser::{ToolSummary, append_text, for_each_jsonl_record};

pub(in crate::commands::agent::watch_web::messages) fn parse(
    reader: impl BufRead,
    from: Option<&str>,
) -> std::io::Result<Vec<ChatItem>> {
    let mut items = Vec::new();
    let mut tools = ToolSummary::default();

    for_each_jsonl_record(reader, |record| {
        let record_type = record.get("type").and_then(Value::as_str);
        let payload = match record_type {
            Some("response_item") | Some("event_msg") => record.get("payload"),
            _ => None,
        };
        let Some(payload) = payload else { return };

        if record_type == Some("response_item") {
            let item_type = payload.get("type").and_then(Value::as_str);
            match item_type {
                Some("message") => {
                    let role = payload.get("role").and_then(Value::as_str);
                    let text = content_text(payload.get("content"));
                    if !text.is_empty() {
                        tools.flush_into(&mut items);
                        append_text(&mut items, text, role == Some("user"), from);
                    }
                }
                Some("function_call") => {
                    let name = payload
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let arguments = payload
                        .get("arguments")
                        .and_then(Value::as_str)
                        .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
                    tools.record_codex(name, arguments.as_ref());
                }
                Some("fileChange") => {
                    tools.record_codex("edit_file", Some(payload));
                }
                _ => {}
            }
        } else if payload.get("type").and_then(Value::as_str) == Some("item_completed")
            && let Some(item) = payload.get("item")
        {
            match item.get("type").and_then(Value::as_str) {
                Some("UserMessage") => {
                    if let Some(text) = item.get("content").and_then(value_text) {
                        tools.flush_into(&mut items);
                        append_text(&mut items, text, true, from);
                    }
                }
                Some("AgentMessage") => {
                    if let Some(text) = item.get("message").and_then(Value::as_str) {
                        tools.flush_into(&mut items);
                        append_text(&mut items, text.to_owned(), false, None);
                    }
                }
                Some("commandExecution") | Some("fileChange") | Some("mcpToolCall") => {
                    tools.record_codex(
                        item.get("type").and_then(Value::as_str).unwrap_or_default(),
                        Some(item),
                    );
                }
                _ => {}
            }
        }
    })?;

    tools.flush_into(&mut items);
    Ok(items)
}

fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| {
                let kind = block.get("type").and_then(Value::as_str)?;
                matches!(kind, "input_text" | "output_text" | "text")
                    .then(|| block.get("text").and_then(Value::as_str))
                    .flatten()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(value) => value_text(value).unwrap_or_default(),
        None => String::new(),
    }
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => Some(
            blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use indoc::indoc;

    use super::parse;
    use crate::commands::agent::watch_web::messages::ChatItem;

    #[test]
    fn parses_rollout_items_and_skips_unknown_rows() {
        let transcript = indoc! {r#"
            {"type":"session_meta","payload":{"id":"fixture-session"}}
            {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":"Summarize these settings."}}}
            {"type":"response_item","payload":{"type":"function_call","name":"apply_patch","arguments":"{\"file_path\":\"src/settings.rs\"}"}}
            {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"fileChange","path":"src/settings.rs"}}}
            {"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"The settings now use the shared parser."}]}}
            {"type":"future_record","payload":{}}
            {not-json
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), Some("fixture-parent-title")).unwrap(),
            vec![
                ChatItem::User {
                    text: "Summarize these settings.".to_string(),
                    from: Some("fixture-parent-title".to_string()),
                },
                ChatItem::ToolSummary {
                    summary: "2 actions · 1 file edited".to_string(),
                },
                ChatItem::Assistant {
                    text: "The settings now use the shared parser.".to_string(),
                },
            ]
        );
    }
}
