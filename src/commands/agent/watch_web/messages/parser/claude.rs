use std::io::BufRead;

use serde_json::Value;

use super::super::ChatItem;
use super::super::ParentSession;
use super::super::parser::{ToolSummary, append_text, for_each_jsonl_record};

pub(in crate::commands::agent::watch_web::messages) fn parse(
    reader: impl BufRead,
    parent: Option<&ParentSession>,
    pending_tool: Option<&str>,
) -> std::io::Result<Vec<ChatItem>> {
    let mut items = Vec::new();
    let mut tools = ToolSummary::default();
    let mut tool_uses = Vec::new();

    for_each_jsonl_record(reader, |record| {
        match record.get("type").and_then(Value::as_str) {
            Some("user") => {
                if let Some(message) = record.get("message") {
                    for id in tool_result_ids(message) {
                        if let Some(index) = tool_uses
                            .iter()
                            .position(|tool_use: &ToolUse| tool_use.id == id)
                        {
                            let tool_use = tool_uses.remove(index);
                            tools.record(&tool_use.name, tool_use.input.as_ref());
                        }
                    }
                    if is_harness_user_record(&record, message) {
                        return;
                    }
                    let text = user_text(message);
                    if !text.is_empty() {
                        tools.flush_into(&mut items);
                        append_text(&mut items, text, true, parent);
                    }
                }
            }
            Some("assistant") => {
                if let Some(content) = record
                    .get("message")
                    .and_then(|message| message.get("content"))
                {
                    for block in content_blocks(content) {
                        match block.get("type").and_then(Value::as_str) {
                            Some("text") => {
                                if let Some(text) = block.get("text").and_then(Value::as_str) {
                                    tools.flush_into(&mut items);
                                    append_text(&mut items, text.to_owned(), false, None);
                                }
                            }
                            Some("tool_use") => {
                                let name = block
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                let input = block.get("input");
                                if name == "AskUserQuestion" {
                                    tools.flush_into(&mut items);
                                    append_questions(&mut items, input);
                                } else if let Some(id) = block.get("id").and_then(Value::as_str) {
                                    tool_uses.push(ToolUse {
                                        id: id.to_owned(),
                                        name: name.to_owned(),
                                        input: input.cloned(),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    })?;

    tools.flush_into(&mut items);
    // Permission state must be tied to an unanswered call ID, not a reused tool name.
    if let Some(tool_use) = pending_tool.and_then(|pending| {
        tool_uses
            .iter()
            .rev()
            .find(|tool_use| matches_pending_tool(tool_use, pending))
    }) {
        items.push(ChatItem::Permission {
            tool: tool_use.name.clone(),
            description: tool_detail(&tool_use.name, tool_use.input.as_ref()),
        });
    }
    Ok(items)
}

struct ToolUse {
    id: String,
    name: String,
    input: Option<Value>,
}

fn tool_result_ids(message: &Value) -> Vec<String> {
    message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
        .filter_map(|block| block.get("tool_use_id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn is_harness_user_record(record: &Value, message: &Value) -> bool {
    ["isMeta", "isSidechain", "isCompactSummary"]
        .into_iter()
        .any(|key| record.get(key).and_then(Value::as_bool) == Some(true))
        || record.get("sourceToolAssistantUUID").is_some()
        || record
            .get("toolUseResult")
            .is_some_and(|result| !result.is_null())
        || [
            "<command-name>",
            "<command-args>",
            "<command-message>",
            "<task-notification>",
            "<local-command-stdout>",
            "<local-command-caveat>",
        ]
        .into_iter()
        .any(|prefix| user_text(message).trim_start().starts_with(prefix))
}

fn matches_pending_tool(tool_use: &ToolUse, pending_tool: &str) -> bool {
    let (name, detail) = pending_tool
        .split_once('(')
        .map_or((pending_tool, None), |(name, detail)| {
            (name, detail.strip_suffix(')'))
        });
    if tool_use.name != name {
        return false;
    }
    let Some(detail) = detail else {
        return true;
    };
    ["command", "file_path", "pattern"]
        .into_iter()
        .find_map(|key| {
            tool_use
                .input
                .as_ref()
                .and_then(|input| input.get(key))
                .and_then(Value::as_str)
        })
        == Some(detail)
}

fn tool_detail(name: &str, input: Option<&Value>) -> String {
    let detail = ["command", "file_path", "path", "pattern", "url", "query"]
        .into_iter()
        .find_map(|key| {
            input
                .and_then(|value| value.get(key))
                .and_then(Value::as_str)
        });
    let Some(detail) = detail else {
        return name.to_owned();
    };
    let detail = detail.chars().take(240).collect::<String>();
    format!("{name}: {detail}")
}

fn user_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) != Some("tool_result"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn content_blocks(content: &Value) -> Vec<&Value> {
    match content {
        Value::Array(blocks) => blocks.iter().collect(),
        Value::String(_) => Vec::new(),
        _ => Vec::new(),
    }
}

fn append_questions(items: &mut Vec<ChatItem>, input: Option<&Value>) {
    let Some(questions) = input
        .and_then(|input| input.get("questions"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for question in questions {
        let Some(text) = question.get("question").and_then(Value::as_str) else {
            continue;
        };
        let choices = question
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|option| option.get("label").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        items.push(ChatItem::Question {
            question: text.to_owned(),
            choices,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use indoc::indoc;

    use super::parse;
    use crate::commands::agent::watch_web::messages::{ChatItem, ParentSession};

    #[test]
    fn parses_messages_tools_questions_and_skips_unknown_rows() {
        let transcript = indoc! {r#"
            {"type":"user","message":{"content":"Review the notification flow."}}
            {"type":"assistant","message":{"content":[{"type":"text","text":"I will inspect the event handler."},{"type":"tool_use","id":"tool-1","name":"Read","input":{"file_path":"fixture/flow.rs"}},{"type":"tool_use","id":"tool-2","name":"Edit","input":{"file_path":"fixture/flow.rs"}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-1","content":"fictional source text"}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-2","content":"fictional edit result"}]}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"tool-3","name":"AskUserQuestion","input":{"questions":[{"question":"Should notifications be grouped?","options":[{"label":"Yes","description":"Group them"},{"label":"No","description":"Show every event"}]}]}}]}}
            {"type":"user","message":{"content":"<peer-message>\n- From session_id: fixture-parent-id\n- Sender engine: fixture-engine\n\nGroup the events by source.\n</peer-message>"}}
            {"type":"user","message":{"content":"<peer-message>\n- From session_id: fixture-other-id\n\nThis came from another child.\n</peer-message>"}}
            {"type":"future_record","payload":{}}
            {not-json
        "#};

        let parent = ParentSession {
            session_id: "fixture-parent-id".to_string(),
            title: "fixture-parent-title".to_string(),
        };

        assert_eq!(
            parse(Cursor::new(transcript), Some(&parent), None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Review the notification flow.".to_string(),
                    from: None,
                },
                ChatItem::Assistant {
                    text: "I will inspect the event handler.".to_string(),
                },
                ChatItem::ToolSummary {
                    summary: "2 actions · 1 file edited".to_string(),
                },
                ChatItem::Question {
                    question: "Should notifications be grouped?".to_string(),
                    choices: vec!["Yes".to_string(), "No".to_string()],
                },
                ChatItem::User {
                    text: "Group the events by source.".to_string(),
                    from: Some("fixture-parent-title".to_string()),
                },
                ChatItem::User {
                    text: "This came from another child.".to_string(),
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn only_unanswered_tool_uses_matching_the_pending_request_become_permissions() {
        let transcript = indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"completed-bash","name":"Bash","input":{"command":"echo earlier"}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"completed-bash","content":"fictional command result"}]}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"pending-bash","name":"Bash","input":{"command":"echo awaiting approval"}}]}}
        "#};

        assert_eq!(
            parse(
                Cursor::new(transcript),
                None,
                Some("Bash(echo awaiting approval)")
            )
            .unwrap(),
            vec![
                ChatItem::ToolSummary {
                    summary: "1 action".to_string(),
                },
                ChatItem::Permission {
                    tool: "Bash".to_string(),
                    description: "Bash: echo awaiting approval".to_string(),
                },
            ]
        );
    }

    #[test]
    fn skips_claude_harness_rows_and_keeps_tool_results_for_action_counts() {
        let transcript = indoc! {r#"
            {"type":"user","isMeta":true,"message":{"content":"Synthetic hook metadata."}}
            {"type":"user","isSidechain":true,"message":{"content":"Synthetic sidechain note."}}
            {"type":"user","isCompactSummary":true,"message":{"content":"Synthetic compact summary."}}
            {"type":"user","message":{"content":"<task-notification>synthetic notification</task-notification>"}}
            {"type":"user","message":{"content":"<local-command-stdout>synthetic output</local-command-stdout>"}}
            {"type":"user","message":{"content":"<local-command-caveat>synthetic caveat</local-command-caveat>"}}
            {"type":"user","message":{"content":"<command-name>/inspect</command-name>\n<command-args>fixture-argument</command-args>\n<command-message>synthetic prompt</command-message>"}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-read","name":"Edit","input":{"file_path":"fixture/notes.txt"}}]}}
            {"type":"user","sourceToolAssistantUUID":"fixture-assistant-id","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-read","content":"synthetic result"}]}}
            {"type":"user","message":{"content":"A visible user message."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None).unwrap(),
            vec![
                ChatItem::ToolSummary {
                    summary: "1 action · 1 file edited".to_string(),
                },
                ChatItem::User {
                    text: "A visible user message.".to_string(),
                    from: None,
                },
            ]
        );
    }
}
