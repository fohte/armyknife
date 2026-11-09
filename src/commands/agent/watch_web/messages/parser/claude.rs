use std::io::BufRead;

use serde_json::Value;

use super::super::ChatItem;
use super::super::parser::{ToolSummary, append_text, for_each_jsonl_record};

pub(in crate::commands::agent::watch_web::messages) fn parse(
    reader: impl BufRead,
    from: Option<&str>,
    pending_tool: Option<&str>,
) -> Vec<ChatItem> {
    let mut items = Vec::new();
    let mut tools = ToolSummary::default();
    let mut pending_permission = None;

    for_each_jsonl_record(reader, |record| {
        match record.get("type").and_then(Value::as_str) {
            Some("user") => {
                if let Some(message) = record.get("message") {
                    let text = user_text(message);
                    if !text.is_empty() {
                        tools.flush_into(&mut items);
                        append_text(&mut items, text, true, from);
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
                                } else if pending_tool.is_some_and(|tool| {
                                    tool.split_once('(').map_or(tool, |(name, _)| name) == name
                                }) {
                                    pending_permission =
                                        Some((name.to_owned(), tool_detail(name, input)));
                                } else {
                                    tools.record(name, input);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    });

    tools.flush_into(&mut items);
    if let Some((tool, description)) = pending_permission {
        items.push(ChatItem::Permission { tool, description });
    }
    items
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
    use crate::commands::agent::watch_web::messages::ChatItem;

    #[test]
    fn parses_messages_tools_questions_and_skips_unknown_rows() {
        let transcript = indoc! {r#"
            {"type":"user","message":{"content":"Review the notification flow."}}
            {"type":"assistant","message":{"content":[{"type":"text","text":"I will inspect the event handler."},{"type":"tool_use","id":"tool-1","name":"Read","input":{"file_path":"src/flow.rs"}},{"type":"tool_use","id":"tool-2","name":"Edit","input":{"file_path":"src/flow.rs"}},{"type":"tool_use","id":"tool-3","name":"AskUserQuestion","input":{"questions":[{"question":"Should notifications be grouped?","options":[{"label":"Yes","description":"Group them"},{"label":"No","description":"Show every event"}]}]}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-1","content":"source text"}]}}
            {"type":"user","message":{"content":"Yes, group them."}}
            {"type":"future_record","payload":{}}
            {not-json
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), Some("fixture-parent-title"), None),
            vec![
                ChatItem::User {
                    text: "Review the notification flow.".to_string(),
                    from: Some("fixture-parent-title".to_string()),
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
                    text: "Yes, group them.".to_string(),
                    from: Some("fixture-parent-title".to_string()),
                },
            ]
        );
    }

    #[test]
    fn includes_the_pending_tool_input_in_permission_messages() {
        let transcript = indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"tool-1","name":"Bash","input":{"command":"cargo test -p sample"}}]}}
        "#};

        assert_eq!(
            parse(
                Cursor::new(transcript),
                None,
                Some("Bash(cargo test -p sample)")
            ),
            vec![ChatItem::Permission {
                tool: "Bash".to_string(),
                description: "Bash: cargo test -p sample".to_string(),
            }]
        );
    }
}
