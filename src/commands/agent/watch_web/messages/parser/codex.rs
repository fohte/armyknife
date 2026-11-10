use std::io::BufRead;

use serde_json::Value;

use super::super::ChatItem;
use super::super::ParentSession;
use super::super::parser::{ToolSummary, append_text, for_each_jsonl_record, operation_target};

pub(in crate::commands::agent::watch_web::messages) fn parse(
    reader: impl BufRead,
    parent: Option<&ParentSession>,
) -> std::io::Result<Vec<ChatItem>> {
    let mut items = Vec::new();
    let mut tools = ToolSummary::default();
    let mut pending_tool_calls: Vec<PendingTool> = Vec::new();

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
                    if let Some(call_id) = payload.get("call_id").and_then(Value::as_str) {
                        pending_tool_calls.push(PendingTool {
                            call_id: call_id.to_owned(),
                            name: name.to_owned(),
                            input,
                        });
                    } else {
                        tools.record_codex(name, input.as_ref());
                    }
                }
                Some("custom_tool_call_output") | Some("function_call_output") => {
                    if let Some(call_id) = payload.get("call_id").and_then(Value::as_str)
                        && let Some(index) = pending_tool_calls
                            .iter()
                            .position(|pending| pending.call_id == call_id)
                    {
                        let completed = pending_tool_calls.remove(index);
                        tools.record_codex(&completed.name, completed.input.as_ref());
                    }
                }
                Some("fileChange") => {
                    if pending_tool_calls.is_empty() {
                        tools.record_codex("edit_file", Some(payload));
                    } else {
                        tools.record_codex_files("edit_file", Some(payload));
                    }
                }
                _ => {}
            }
        } else if matches!(
            payload.get("type").and_then(Value::as_str),
            Some("task_complete" | "turn_aborted")
        ) {
            // Missing tool outputs must not suppress actions from the next turn.
            for pending in pending_tool_calls.drain(..) {
                tools.record_codex(&pending.name, pending.input.as_ref());
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
                    let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
                    if pending_tool_calls.is_empty() {
                        tools.record_codex(item_type, Some(item));
                    } else {
                        tools.record_codex_files(item_type, Some(item));
                    }
                }
                _ => {}
            }
        }
    })?;

    let active_tool = pending_tool_calls
        .last()
        .map(|pending| ChatItem::ToolInProgress {
            tool: pending.name.clone(),
            target: operation_target(&pending.name, pending.input.as_ref()),
        });
    tools.flush_into(&mut items);
    if let Some(active_tool) = active_tool {
        items.push(active_tool);
    }
    Ok(items)
}

struct PendingTool {
    call_id: String,
    name: String,
    input: Option<Value>,
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
        Some(Value::String(raw)) => match serde_json::from_str(raw) {
            Ok(Value::Object(value)) => Some(Value::Object(value)),
            _ => Some(Value::String(raw.clone())),
        },
        Some(value) => Some(value.clone()),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use indoc::indoc;
    use rstest::rstest;
    use serde_json::json;

    use super::parse;
    use crate::commands::agent::watch_web::messages::{ChatItem, ParentSession};

    #[test]
    fn parses_rollout_items_and_skips_unknown_rows() {
        let transcript = indoc! {r#"
            {"type":"session_meta","payload":{"id":"fixture-session"}}
            {"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Synthetic AGENTS instructions."}]}}
            {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"Text","text":"Summarize these settings."}]}}}
            {"type":"response_item","payload":{"type":"custom_tool_call","call_id":"fixture-call","name":"exec","input":"printf 'fictional command output'"}}
            {"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"fixture-call","output":"fixture result"}}
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
                    summary: "1 action".to_string(),
                    operations: vec![operation("exec", Some("printf 'fictional command output'"))],
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
                operations: vec![operation("CommandExecution", Some("echo fixture"))],
            }]
        );
    }

    #[test]
    fn does_not_count_command_execution_again_for_an_open_custom_tool_call() {
        let transcript = indoc! {r#"
            {"type":"response_item","payload":{"type":"custom_tool_call","call_id":"fixture-call-id","name":"exec","input":"printf 'fictional command output'"}}
            {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","id":"fixture-command-id","command":"echo fixture","cwd":"/fixture/project","duration":1,"exit_code":0,"status":"completed"}}}
            {"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"fixture-call-id","output":"fixture output"}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None).unwrap(),
            vec![ChatItem::ToolSummary {
                summary: "1 action".to_string(),
                operations: vec![operation("exec", Some("printf 'fictional command output'"))],
            }]
        );
    }

    #[test]
    fn counts_file_changes_once_and_uses_object_keys_for_edited_files() {
        let transcript = indoc! {r#"
            {"type":"response_item","payload":{"type":"custom_tool_call","call_id":"fixture-call-id","name":"exec","input":"printf 'fictional command output'"}}
            {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"FileChange","changes":{"src/fixture-one.rs":{"type":"update","content":"fictional change content"},"src/fixture-two.rs":{"type":"update","unified_diff":"fictional diff","move_path":null}}}}}
            {"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"fixture-call-id","output":"fictional output"}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None).unwrap(),
            vec![ChatItem::ToolSummary {
                summary: "1 action · 2 files edited".to_string(),
                operations: vec![operation("exec", Some("printf 'fictional command output'"))],
            }]
        );
    }

    #[test]
    fn counts_file_changes_outside_a_custom_tool_call_as_an_action() {
        let transcript = indoc! {r#"
            {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"FileChange","changes":{"src/fixture.rs":{"type":"update","unified_diff":"fictional diff","move_path":null}}}}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None).unwrap(),
            vec![ChatItem::ToolSummary {
                summary: "1 action · 1 file edited".to_string(),
                operations: vec![operation("FileChange", Some("src/fixture.rs"))],
            }]
        );
    }

    #[rstest]
    #[case::aborted("turn_aborted")]
    #[case::completed("task_complete")]
    fn terminal_turn_events_clear_unanswered_tool_calls(#[case] event_type: &str) {
        let transcript = [
            json!({
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call",
                    "call_id": "fixture-unanswered-call",
                    "name": "exec",
                    "input": "printf 'fictional command output'",
                },
            }),
            json!({"type": "event_msg", "payload": {"type": event_type}}),
            json!({
                "type": "event_msg",
                "payload": {
                    "type": "item_completed",
                    "item": {
                        "type": "CommandExecution",
                        "id": "fixture-command-id",
                        "command": "printf fixture",
                        "cwd": "/fixture/project",
                        "duration": 1,
                        "exit_code": 0,
                        "status": "completed",
                    },
                },
            }),
            json!({
                "type": "event_msg",
                "payload": {
                    "type": "item_completed",
                    "item": {
                        "type": "FileChange",
                        "changes": {
                            "src/fixture.rs": {
                                "type": "update",
                                "unified_diff": "fictional diff",
                                "move_path": null,
                            },
                        },
                    },
                },
            }),
        ]
        .into_iter()
        .map(|record| serde_json::to_string(&record).unwrap())
        .collect::<Vec<_>>()
        .join("\n");

        assert_eq!(
            parse(Cursor::new(transcript), None).unwrap(),
            vec![ChatItem::ToolSummary {
                summary: "3 actions · 1 file edited".to_string(),
                operations: vec![
                    operation("exec", Some("printf 'fictional command output'")),
                    operation("CommandExecution", Some("printf fixture")),
                    operation("FileChange", Some("src/fixture.rs")),
                ],
            }]
        );
    }

    #[rstest]
    #[case::plain_command("echo fixture")]
    #[case::json_boolean_literal("true")]
    fn raw_exec_input_is_preserved_as_a_command_target(#[case] command: &str) {
        let transcript = [
            json!({
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call",
                    "call_id": "fixture-command-call",
                    "name": "exec",
                    "input": command,
                },
            }),
            json!({
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call_output",
                    "call_id": "fixture-command-call",
                    "output": "fixture result",
                },
            }),
        ]
        .into_iter()
        .map(|record| serde_json::to_string(&record).unwrap())
        .collect::<Vec<_>>()
        .join("\n");

        assert_eq!(
            parse(Cursor::new(transcript), None).unwrap(),
            vec![ChatItem::ToolSummary {
                summary: "1 action".to_string(),
                operations: vec![operation("exec", Some(command))],
            }]
        );
    }

    #[test]
    fn returns_the_last_tool_call_without_a_result() {
        let transcript = indoc! {r#"
            {"type":"response_item","payload":{"type":"custom_tool_call","call_id":"fixture-completed","name":"exec","input":"printf done"}}
            {"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"fixture-completed","output":"fixture result"}}
            {"type":"response_item","payload":{"type":"function_call","call_id":"fixture-active-one","name":"read_file","arguments":"{\"path\":\"fixture/one.rs\"}"}}
            {"type":"response_item","payload":{"type":"function_call","call_id":"fixture-active-two","name":"exec","arguments":"cargo test"}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None).unwrap(),
            vec![
                ChatItem::ToolSummary {
                    summary: "1 action".to_string(),
                    operations: vec![operation("exec", Some("printf done"))],
                },
                ChatItem::ToolInProgress {
                    tool: "exec".to_string(),
                    target: Some("cargo test".to_string()),
                },
            ]
        );
    }

    #[test]
    fn parses_mcp_tool_calls_with_their_name_and_target() {
        let transcript = indoc! {r#"
            {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"McpToolCall","tool":"read_resource","arguments":{"path":"fixture/resource.json"}}}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None).unwrap(),
            vec![ChatItem::ToolSummary {
                summary: "1 action".to_string(),
                operations: vec![operation("read_resource", Some("fixture/resource.json"))],
            }]
        );
    }

    fn operation(tool: &str, target: Option<&str>) -> super::super::super::ToolOperation {
        super::super::super::ToolOperation {
            tool: tool.to_string(),
            target: target.map(str::to_owned),
        }
    }
}
