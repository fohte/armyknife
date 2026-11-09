use std::io::BufRead;

use serde_json::Value;

use super::super::ChatItem;
use super::super::ParentSession;
use super::super::parser::{ToolSummary, append_text, for_each_jsonl_record};

pub(in crate::commands::agent::watch_web::messages) fn parse(
    reader: impl BufRead,
    parent: Option<&ParentSession>,
) -> std::io::Result<Vec<ChatItem>> {
    let mut items = Vec::new();
    let mut tools = ToolSummary::default();
    let mut pending_tool_calls = Vec::new();

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
                    // User role records can include injected instructions; UserMessage is the visible source.
                    if role == Some("assistant") {
                        let text = content_text(payload.get("content"));
                        if !text.is_empty() {
                            tools.flush_into(&mut items);
                            append_text(&mut items, text, false, None);
                        }
                    }
                }
                Some("custom_tool_call") | Some("function_call") => {
                    let name = payload
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let input =
                        parse_tool_input(payload.get("input").or_else(|| payload.get("arguments")));
                    tools.record_codex(name, input.as_ref());
                    if let Some("custom_tool_call" | "function_call") = item_type
                        && let Some(call_id) = payload.get("call_id").and_then(Value::as_str)
                    {
                        pending_tool_calls.push(call_id.to_owned());
                    }
                }
                Some("custom_tool_call_output") | Some("function_call_output") => {
                    if let Some(call_id) = payload.get("call_id").and_then(Value::as_str)
                        && let Some(index) = pending_tool_calls
                            .iter()
                            .position(|pending| pending == call_id)
                    {
                        pending_tool_calls.remove(index);
                    }
                }
                Some("fileChange") => {
                    tools.record_codex("edit_file", Some(payload));
                }
                _ => {}
            }
        } else if payload.get("type").and_then(Value::as_str) == Some("item_completed")
            && let Some(item) = payload.get("item")
        {
            // AgentMessage repeats the assistant response_item and would duplicate its bubble.
            match item.get("type").and_then(Value::as_str) {
                Some("UserMessage") => {
                    if let Some(text) = item.get("content").and_then(value_text) {
                        tools.flush_into(&mut items);
                        append_text(&mut items, text, true, parent);
                    }
                }
                Some("CommandExecution") | Some("commandExecution") => {
                    // CommandExecution can describe work performed by an open custom call.
                    if pending_tool_calls.is_empty() {
                        tools.record_codex(
                            item.get("type").and_then(Value::as_str).unwrap_or_default(),
                            Some(item),
                        );
                    }
                }
                Some("FileChange") | Some("McpToolCall") | Some("fileChange")
                | Some("mcpToolCall") => {
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

fn parse_tool_input(input: Option<&Value>) -> Option<Value> {
    match input {
        Some(Value::String(raw)) => serde_json::from_str(raw).ok(),
        Some(value) => Some(value.clone()),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use indoc::indoc;

    use super::parse;
    use crate::commands::agent::watch_web::messages::{ChatItem, ParentSession};

    #[test]
    fn parses_rollout_items_and_skips_unknown_rows() {
        let transcript = indoc! {r#"
            {"type":"session_meta","payload":{"id":"fixture-session"}}
            {"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Synthetic AGENTS instructions."}]}}
            {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"Text","text":"Summarize these settings."}]}}}
            {"type":"response_item","payload":{"type":"custom_tool_call","call_id":"fixture-call","name":"apply_patch","input":"{\"file_path\":\"fixture/settings.toml\"}"}}
            {"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"The settings now use the shared parser."}]}}
            {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","content":[{"type":"Text","text":"The settings now use the shared parser."}]}}}
            {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"Text","text":"<peer-message>\n- From session_id: fixture-parent-id\n\nPlease keep the summary brief.\n</peer-message>"}]}}}
            {"type":"future_record","payload":{}}
            {not-json
        "#};

        let parent = ParentSession {
            session_id: "fixture-parent-id".to_string(),
            title: "fixture-parent-title".to_string(),
        };

        assert_eq!(
            parse(Cursor::new(transcript), Some(&parent)).unwrap(),
            vec![
                ChatItem::User {
                    text: "Summarize these settings.".to_string(),
                    from: None,
                },
                ChatItem::ToolSummary {
                    summary: "1 action · 1 file edited".to_string(),
                },
                ChatItem::Assistant {
                    text: "The settings now use the shared parser.".to_string(),
                },
                ChatItem::User {
                    text: "Please keep the summary brief.".to_string(),
                    from: Some("fixture-parent-title".to_string()),
                },
            ]
        );
    }

    #[test]
    fn parses_command_execution_items_with_the_rollout_type_name() {
        let transcript = indoc! {r#"
            {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","id":"fixture-command","command":"echo fixture","cwd":"/fixture/project","duration":1,"exit_code":0,"status":"completed"}}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None).unwrap(),
            vec![ChatItem::ToolSummary {
                summary: "1 action".to_string(),
            }]
        );
    }

    #[test]
    fn does_not_count_command_execution_again_for_an_open_custom_tool_call() {
        let transcript = indoc! {r#"
            {"type":"response_item","payload":{"type":"custom_tool_call","call_id":"fixture-call-id","name":"run_command","input":"{}"}}
            {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","id":"fixture-command-id","command":"echo fixture","cwd":"/fixture/project","duration":1,"exit_code":0,"status":"completed"}}}
            {"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"fixture-call-id","output":"fixture output"}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None).unwrap(),
            vec![ChatItem::ToolSummary {
                summary: "1 action".to_string(),
            }]
        );
    }
}
