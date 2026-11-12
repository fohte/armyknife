use std::io::BufRead;

use serde_json::Value;

use super::super::ChatItem;
use super::super::ParentSession;
use super::super::SkillSuggestion;
use super::super::parser::{
    ToolSummary, append_text, for_each_jsonl_record, operation_target, record_timestamp,
};
use super::decision_position::{DecisionTimeline, insert_time_placed_decisions};
use crate::commands::agent::store::AgentDecision;

#[cfg(test)]
fn parse(reader: impl BufRead, parent: Option<&ParentSession>) -> std::io::Result<Vec<ChatItem>> {
    parse_with_skills(reader, parent, &[]).map(|(messages, _)| messages)
}

pub(in crate::commands::agent::watch_web::messages) fn parse_with_skills(
    reader: impl BufRead,
    parent: Option<&ParentSession>,
    decisions: &[AgentDecision],
) -> std::io::Result<(Vec<ChatItem>, Vec<SkillSuggestion>)> {
    let mut items = Vec::new();
    let mut tools = ToolSummary::default();
    let mut pending_tool_calls: Vec<PendingTool> = Vec::new();
    let mut skills = Vec::new();
    let mut decision_timeline = DecisionTimeline::new(decisions);

    for_each_jsonl_record(reader, |record| {
        super::skill_suggestions::capture_codex_listing(&record, &mut skills);
        let timestamp = record_timestamp(&record);
        insert_time_placed_decisions(timestamp, &mut decision_timeline, &mut tools, &mut items);
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
                            append_text(&mut items, text, false, None, timestamp);
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
                        tools.record_codex(name, input.as_ref(), false);
                    }
                }
                Some("custom_tool_call_output") | Some("function_call_output") => {
                    if let Some(call_id) = payload.get("call_id").and_then(Value::as_str)
                        && let Some(index) = pending_tool_calls
                            .iter()
                            .position(|pending| pending.call_id == call_id)
                    {
                        let completed = pending_tool_calls.remove(index);
                        tools.record_codex(
                            &completed.name,
                            completed.input.as_ref(),
                            codex_tool_failed(payload),
                        );
                    }
                }
                Some("fileChange") => {
                    if pending_tool_calls.is_empty() {
                        tools.record_codex("edit_file", Some(payload), codex_tool_failed(payload));
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
            let failed = payload.get("type").and_then(Value::as_str) == Some("turn_aborted");
            for pending in pending_tool_calls.drain(..) {
                tools.record_codex(&pending.name, pending.input.as_ref(), failed);
            }
        } else if payload.get("type").and_then(Value::as_str) == Some("item_completed")
            && let Some(item) = payload.get("item")
        {
            // AgentMessage repeats the assistant response_item and would duplicate its bubble.
            match item.get("type").and_then(Value::as_str) {
                Some("UserMessage") => {
                    if let Some(text) = item.get("content").and_then(value_text) {
                        tools.flush_into(&mut items);
                        append_text(&mut items, text, true, parent, timestamp);
                    }
                }
                Some("CommandExecution") | Some("commandExecution") => {
                    // CommandExecution can describe work performed by an open custom call.
                    if pending_tool_calls.is_empty() {
                        tools.record_codex(
                            item.get("type").and_then(Value::as_str).unwrap_or_default(),
                            Some(item),
                            codex_tool_failed(item),
                        );
                    }
                }
                Some("FileChange") | Some("McpToolCall") | Some("fileChange")
                | Some("mcpToolCall") => {
                    let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
                    if pending_tool_calls.is_empty() {
                        tools.record_codex(item_type, Some(item), codex_tool_failed(item));
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
    decision_timeline.append_remaining(&mut items);
    Ok((items, skills))
}

fn codex_tool_failed(result: &Value) -> bool {
    let values = [Some(result), result.get("output")];
    values.into_iter().flatten().any(|value| {
        ["is_error", "isError", "failed"]
            .into_iter()
            .any(|key| value.get(key).and_then(Value::as_bool) == Some(true))
            || value
                .get("exit_code")
                .and_then(Value::as_i64)
                .is_some_and(|exit_code| exit_code != 0)
            || value
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|status| matches!(status, "failed" | "error" | "aborted"))
    })
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

    use super::{parse, parse_with_skills};
    use crate::commands::agent::watch_web::messages::{
        ChatItem, ParentSession, PeerDirection, SkillSuggestion, UserContentPart,
    };

    #[test]
    fn parses_the_latest_injected_skill_listing() {
        let transcript = indoc! {r###"
            {"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<skills_instructions>\n## Skills\n### Available skills\n- old-tool: Old description.\n</skills_instructions>"}]}}
            {"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<skills_instructions>\n## Skills\n### Available skills\n- design-flow: Help shape a design.\n  location: /fixture/path\n- debug-flow: Investigate a failure.\n</skills_instructions>"}]}}
            {"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"## Skills\n### Available skills\n- nonexistent-tool: Quoted by the user."}]}}
        "###};

        assert_eq!(
            parse_with_skills(Cursor::new(transcript), None, &[]).unwrap(),
            (
                Vec::new(),
                vec![
                    SkillSuggestion {
                        name: "design-flow".to_string(),
                        description: "Help shape a design.".to_string(),
                    },
                    SkillSuggestion {
                        name: "debug-flow".to_string(),
                        description: "Investigate a failure.".to_string(),
                    },
                ],
            ),
        );
    }

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
                    timestamp: None,
                    from: None,
                },
                ChatItem::ToolSummary {
                    summary: "exec ×1".to_string(),
                    operations: vec![operation("exec", Some("printf 'fictional command output'"))],
                },
                ChatItem::Assistant {
                    text: "The settings now use the shared parser.".to_string(),
                },
                ChatItem::Peer {
                    direction: PeerDirection::From,
                    session_id: "fixture-parent-id".to_string(),
                    session_title: Some("fixture-parent-title".to_string()),
                    timestamp: None,
                    content: vec![UserContentPart::Text {
                        text: "Please keep the summary brief.".to_string(),
                    }],
                },
            ]
        );
    }

    #[test]
    fn records_timestamps_for_user_messages_without_marking_assistant_text() {
        let transcript = indoc! {r#"
            {"type":"event_msg","timestamp":"2026-11-11T10:00:00Z","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"Text","text":"Human request."}]}}}
            {"type":"response_item","timestamp":"2026-11-11T10:01:00Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Assistant response."}]}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Human request.".to_string(),
                    from: None,
                    timestamp: Some("2026-11-11T10:00:00+00:00".to_string()),
                },
                ChatItem::Assistant {
                    text: "Assistant response.".to_string(),
                },
            ]
        );
    }

    #[rstest]
    #[case::successful_exit(0, false)]
    #[case::nonzero_exit(1, true)]
    fn parses_command_execution_exit_codes(#[case] exit_code: i32, #[case] failed: bool) {
        let transcript = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "item_completed",
                "item": {
                    "type": "CommandExecution",
                    "id": "fixture-command",
                    "command": "echo fixture",
                    "cwd": "/fixture/project",
                    "duration": 1,
                    "exit_code": exit_code,
                },
            },
        })
        .to_string();

        assert_eq!(
            parse(Cursor::new(transcript), None).unwrap(),
            vec![ChatItem::ToolSummary {
                summary: if failed {
                    "CommandExecution ×1 · 1 failed".to_string()
                } else {
                    "CommandExecution ×1".to_string()
                },
                operations: vec![operation_with_failure(
                    "CommandExecution",
                    Some("echo fixture"),
                    failed,
                )],
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
                summary: "exec ×1".to_string(),
                operations: vec![operation("exec", Some("printf 'fictional command output'"))],
            }]
        );
    }

    #[test]
    fn marks_failed_custom_tool_call_outputs() {
        let transcript = indoc! {r#"
            {"type":"response_item","payload":{"type":"custom_tool_call","call_id":"fixture-call-id","name":"exec","input":"false"}}
            {"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"fixture-call-id","output":"fixture failure","is_error":true}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None).unwrap(),
            vec![ChatItem::ToolSummary {
                summary: "exec ×1 · 1 failed".to_string(),
                operations: vec![operation_with_failure("exec", Some("false"), true)],
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
                summary: "Edit fixture-one.rs, fixture-two.rs  exec ×1".to_string(),
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
                summary: "Edit fixture.rs".to_string(),
                operations: vec![operation("FileChange", Some("src/fixture.rs"))],
            }]
        );
    }

    #[rstest]
    #[case::aborted("turn_aborted", true)]
    #[case::completed("task_complete", false)]
    fn terminal_turn_events_clear_unanswered_tool_calls(
        #[case] event_type: &str,
        #[case] tool_failed: bool,
    ) {
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
                summary: if tool_failed {
                    "Edit fixture.rs  exec ×1  CommandExecution ×1 · 1 failed".to_string()
                } else {
                    "Edit fixture.rs  exec ×1  CommandExecution ×1".to_string()
                },
                operations: vec![
                    operation_with_failure(
                        "exec",
                        Some("printf 'fictional command output'"),
                        tool_failed,
                    ),
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
                summary: "exec ×1".to_string(),
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
                    summary: "exec ×1".to_string(),
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
                summary: "read_resource ×1".to_string(),
                operations: vec![operation("read_resource", Some("fixture/resource.json"))],
            }]
        );
    }

    fn operation(tool: &str, target: Option<&str>) -> super::super::super::ToolOperation {
        operation_with_failure(tool, target, false)
    }

    fn operation_with_failure(
        tool: &str,
        target: Option<&str>,
        failed: bool,
    ) -> super::super::super::ToolOperation {
        super::super::super::ToolOperation {
            tool: tool.to_string(),
            target: target.map(str::to_owned),
            failed,
        }
    }
}
