use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, Seek, SeekFrom};

use serde_json::Value;

use super::super::ChatItem;
use super::super::ParentSession;
use super::super::parser::{
    ToolSummary, append_text, append_text_with_user_marker, for_each_jsonl_record,
    peer_message_item_with_user_marker, tool_target,
};

pub(in crate::commands::agent::watch_web::messages) fn parse(
    mut reader: impl BufRead + Seek,
    parent: Option<&ParentSession>,
    pending_tool: Option<&str>,
    user_marker: Option<&str>,
) -> std::io::Result<Vec<ChatItem>> {
    let mut transcript_nodes = BTreeMap::new();
    let mut latest_uuid = None;
    for_each_jsonl_record(&mut reader, |record| {
        let Some(uuid) = record.get("uuid").and_then(Value::as_str) else {
            return;
        };
        latest_uuid = Some(uuid.to_owned());
        let parent_uuid = match record.get("parentUuid") {
            Some(Value::String(parent)) => Some(parent.clone()),
            Some(Value::Null) => match record.get("logicalParentUuid") {
                None | Some(Value::Null) => None,
                Some(Value::String(parent)) => Some(parent.clone()),
                _ => None,
            },
            _ => None,
        };
        transcript_nodes.insert(
            uuid.to_owned(),
            TranscriptNode {
                parent_uuid,
                membership: BranchMembership::Unknown,
            },
        );
    })?;
    mark_active_branch(&mut transcript_nodes, latest_uuid.as_deref());
    reader.seek(SeekFrom::Start(0))?;
    let mut items = Vec::new();
    let mut tools = ToolSummary::default();
    let mut tool_uses = Vec::new();
    let mut question_uses = HashMap::<String, Vec<usize>>::new();

    for_each_jsonl_record(&mut reader, |record| {
        if let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
            let membership = transcript_nodes.get(uuid).map(|node| node.membership);
            let is_tool_result_only = is_tool_result_only_user_record(&record);
            if membership.is_none() && !is_tool_result_only {
                return;
            }
            if membership == Some(BranchMembership::Unknown)
                && !is_tool_result_only
                && is_inactive_branch(uuid, &mut transcript_nodes)
            {
                return;
            }
        }
        (|| match record.get("type").and_then(Value::as_str) {
            _ if record
                .get("attachment")
                .and_then(|attachment| attachment.get("type"))
                .and_then(Value::as_str)
                == Some("queued_command") =>
            {
                if let Some(prompt) = record
                    .get("attachment")
                    .and_then(|attachment| attachment.get("prompt"))
                {
                    let text = content_text(Some(prompt));
                    if !text.is_empty() {
                        tools.flush_into(&mut items);
                        append_text_with_user_marker(&mut items, text, parent, user_marker);
                    }
                }
            }
            Some("user") => {
                if let Some(message) = record.get("message") {
                    let tool_use_result = record
                        .get("toolUseResult")
                        .filter(|result| !result.is_null());
                    for block in tool_result_blocks(message) {
                        let Some(id) = block.get("tool_use_id").and_then(Value::as_str) else {
                            continue;
                        };
                        if let Some(indices) = question_uses.remove(id) {
                            let answers = match tool_use_result {
                                Some(result) => tool_use_result_answers(result),
                                None => block
                                    .get("content")
                                    .and_then(tool_result_text)
                                    .and_then(|text| parse_question_answers(&text)),
                            };
                            if let Some(answers) = answers {
                                apply_question_answers(&mut items, &indices, &answers);
                            }
                        }
                        if let Some(index) = tool_uses
                            .iter()
                            .position(|tool_use: &ToolUse| tool_use.id == id)
                        {
                            let tool_use = tool_uses.remove(index);
                            tools.record(&tool_use.name, tool_use.input.as_ref());
                        }
                    }
                    let Some(origin) = user_origin(&record, message) else {
                        return;
                    };
                    let text = user_text(message);
                    if !text.is_empty() {
                        if origin == UserOrigin::Peer {
                            let Some(item) =
                                peer_message_item_with_user_marker(&text, parent, user_marker)
                            else {
                                return;
                            };
                            tools.flush_into(&mut items);
                            items.push(item);
                        } else {
                            tools.flush_into(&mut items);
                            append_text(
                                &mut items,
                                slash_command_text(&text).unwrap_or(text),
                                true,
                                parent,
                            );
                        }
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
                                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                                        let indices = append_questions(&mut items, input);
                                        question_uses.insert(id.to_owned(), indices);
                                    } else {
                                        append_questions(&mut items, input);
                                    }
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
        })();
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
    } else if let Some(tool_use) = tool_uses.last() {
        items.push(ChatItem::ToolInProgress {
            tool: tool_use.name.clone(),
            target: tool_target(tool_use.input.as_ref()),
        });
    }
    Ok(items)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BranchMembership {
    Unknown,
    Visiting,
    Active,
    ReachesActive,
    Disconnected,
}

struct TranscriptNode {
    parent_uuid: Option<String>,
    membership: BranchMembership,
}

fn mark_active_branch(
    transcript_nodes: &mut BTreeMap<String, TranscriptNode>,
    latest_uuid: Option<&str>,
) {
    let Some(mut uuid) = latest_uuid.map(str::to_owned) else {
        return;
    };

    loop {
        let Some(node) = transcript_nodes.get_mut(&uuid) else {
            return;
        };
        if node.membership == BranchMembership::Active {
            return;
        }
        node.membership = BranchMembership::Active;
        let Some(parent_uuid) = &node.parent_uuid else {
            return;
        };
        let parent_uuid = parent_uuid.clone();
        if !transcript_nodes.contains_key(&parent_uuid) {
            return;
        }
        uuid = parent_uuid;
    }
}

fn is_inactive_branch(uuid: &str, transcript_nodes: &mut BTreeMap<String, TranscriptNode>) -> bool {
    let mut current_uuid = uuid.to_owned();
    let mut path = Vec::new();
    let reaches_active = loop {
        let Some(node) = transcript_nodes.get_mut(&current_uuid) else {
            break false;
        };
        match node.membership {
            BranchMembership::Active | BranchMembership::ReachesActive => break true,
            BranchMembership::Disconnected | BranchMembership::Visiting => break false,
            BranchMembership::Unknown => node.membership = BranchMembership::Visiting,
        }
        path.push(current_uuid.clone());
        let Some(parent_uuid) = &node.parent_uuid else {
            break false;
        };
        current_uuid = parent_uuid.clone();
    };

    let membership = if reaches_active {
        BranchMembership::ReachesActive
    } else {
        BranchMembership::Disconnected
    };
    for uuid in path {
        if let Some(node) = transcript_nodes.get_mut(&uuid) {
            node.membership = membership;
        }
    }
    reaches_active
}

struct ToolUse {
    id: String,
    name: String,
    input: Option<Value>,
}

fn tool_result_blocks(message: &Value) -> impl Iterator<Item = &Value> {
    message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
}

fn tool_result_text(content: &Value) -> Option<String> {
    match content {
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

fn parse_question_answers(content: &str) -> Option<Vec<(String, String)>> {
    let mut remaining = content
        .strip_prefix("Your questions have been answered:")
        .or_else(|| content.strip_prefix("The user answered:"))?
        .trim_start();
    let mut answers = Vec::new();

    loop {
        let (question, after_question) = parse_quoted_json_string(remaining)?;
        let after_equals = after_question.strip_prefix('=')?;
        let (answer, after_answer) = parse_quoted_json_string(after_equals)?;
        answers.push((question, answer));

        remaining = after_answer.trim_start();
        if let Some(after_comma) = remaining.strip_prefix(',') {
            remaining = after_comma.trim_start();
        } else if !remaining.starts_with('"') {
            break;
        }
    }

    (!answers.is_empty()).then_some(answers)
}

fn tool_use_result_answers(tool_use_result: &Value) -> Option<Vec<(String, String)>> {
    let answers = tool_use_result.get("answers")?.as_object()?;
    let answers = answers
        .iter()
        .filter_map(|(question, answer)| {
            answer
                .as_str()
                .map(|answer| (question.clone(), answer.to_owned()))
        })
        .collect::<Vec<_>>();
    (!answers.is_empty()).then_some(answers)
}

fn parse_quoted_json_string(value: &str) -> Option<(String, &str)> {
    if !value.starts_with('"') {
        return None;
    }

    let mut escaped = false;
    for (index, character) in value.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            let end = index + character.len_utf8();
            let parsed = serde_json::from_str(&value[..end]).ok()?;
            return Some((parsed, &value[end..]));
        }
    }

    None
}

fn apply_question_answers(
    items: &mut [ChatItem],
    question_indices: &[usize],
    answers: &[(String, String)],
) {
    let mut remaining_answers = answers.to_vec();
    for index in question_indices {
        let Some(ChatItem::Question {
            question, answer, ..
        }) = items.get_mut(*index)
        else {
            continue;
        };
        if let Some(answer_index) = remaining_answers
            .iter()
            .position(|(asked, _)| asked == question)
        {
            let (_, selected_answer) = remaining_answers.remove(answer_index);
            *answer = Some(selected_answer);
        }
    }
}

fn is_tool_result_only_user_record(record: &Value) -> bool {
    record.get("type").and_then(Value::as_str) == Some("user")
        && record
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                !blocks.is_empty()
                    && blocks.iter().all(|block| {
                        block.get("type").and_then(Value::as_str) == Some("tool_result")
                    })
            })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UserOrigin {
    Human,
    Peer,
    Legacy,
}

fn user_origin(record: &Value, message: &Value) -> Option<UserOrigin> {
    if record.get("sourceToolAssistantUUID").is_some()
        || record
            .get("toolUseResult")
            .is_some_and(|result| !result.is_null())
    {
        return None;
    }

    if let Some(origin) = record.get("origin").filter(|origin| !origin.is_null()) {
        return match origin.get("kind").and_then(Value::as_str) {
            Some("human") => Some(UserOrigin::Human),
            Some("peer") => Some(UserOrigin::Peer),
            _ => None,
        };
    }

    (!is_legacy_harness_user_record(record, message)).then_some(UserOrigin::Legacy)
}

fn is_legacy_harness_user_record(record: &Value, message: &Value) -> bool {
    let text = user_text(message);
    ["isMeta", "isSidechain", "isCompactSummary"]
        .into_iter()
        .any(|key| record.get(key).and_then(Value::as_bool) == Some(true))
        || (text.trim_start().starts_with("<command-message>") && !text.contains("<command-name>"))
        || [
            "<task-notification>",
            "<local-command-stdout>",
            "<local-command-caveat>",
        ]
        .into_iter()
        .any(|prefix| text.trim_start().starts_with(prefix))
}

fn slash_command_text(text: &str) -> Option<String> {
    let command = text
        .split_once("<command-name>")?
        .1
        .split_once("</command-name>")?
        .0
        .trim();
    if command.is_empty() {
        return None;
    }
    let command = if command.starts_with('/') {
        command.to_owned()
    } else {
        format!("/{command}")
    };
    let args = text
        .split_once("<command-args>")
        .and_then(|(_, value)| value.split_once("</command-args>"))
        .map_or("", |(args, _)| args.trim());
    Some(if args.is_empty() {
        command
    } else {
        format!("{command} {args}")
    })
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
    let Some(target) = tool_target(input) else {
        return name.to_owned();
    };
    format!("{name}: {target}")
}

fn user_text(message: &Value) -> String {
    content_text(message.get("content"))
}

fn content_text(content: Option<&Value>) -> String {
    match content {
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

fn append_questions(items: &mut Vec<ChatItem>, input: Option<&Value>) -> Vec<usize> {
    let Some(questions) = input
        .and_then(|input| input.get("questions"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut indices = Vec::new();
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
        indices.push(items.len());
        items.push(ChatItem::Question {
            question: text.to_owned(),
            choices,
            answer: None,
        });
    }
    indices
}

#[cfg(test)]
mod tests {
    use std::io::{self, BufRead, Cursor, Read, Seek, SeekFrom};

    use indoc::indoc;
    use rstest::rstest;

    use super::parse;
    use crate::commands::agent::watch_web::messages::{ChatItem, ParentSession};

    #[rstest]
    #[case::prompt_array(
        indoc! {r#"
            {"type":"user","message":{"content":"Before the queued prompt."}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-tool-id","name":"Read","input":{}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-tool-id","content":"Synthetic result."}]}}
            {"type":"queue-operation","operation":"enqueue","content":"Show this prompt once."}
            {"parentUuid":"fixture-turn-id","attachment":{"type":"queued_command","prompt":[{"type":"text","text":"Show this prompt once."},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"c3ludGhldGlj"}}]}}
            {"type":"queue-operation","operation":"remove","reason":"absorbed_mid_turn","content":"Show this prompt once."}
            {"type":"user","message":{"content":"After the queued prompt."}}
        "#},
        "Show this prompt once."
    )]
    #[case::prompt_string(
        indoc! {r#"
            {"type":"user","message":{"content":"Before the queued prompt."}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-tool-id","name":"Read","input":{}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-tool-id","content":"Synthetic result."}]}}
            {"type":"queue-operation","operation":"enqueue","content":"Show this prompt once."}
            {"parentUuid":"fixture-turn-id","attachment":{"type":"queued_command","prompt":"Show this prompt once."}}
            {"type":"queue-operation","operation":"remove","reason":"absorbed_mid_turn","content":"Show this prompt once."}
            {"type":"user","message":{"content":"After the queued prompt."}}
        "#},
        "Show this prompt once."
    )]
    fn queued_commands_are_shown_once_in_transcript_order(
        #[case] transcript: &str,
        #[case] queued_text: &str,
    ) {
        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Before the queued prompt.".to_string(),
                    from: None,
                },
                ChatItem::ToolSummary {
                    summary: "1 action".to_string(),
                    operations: vec![operation("Read", None)],
                },
                ChatItem::User {
                    text: queued_text.to_string(),
                    from: None,
                },
                ChatItem::User {
                    text: "After the queued prompt.".to_string(),
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn queued_peer_messages_use_the_existing_sender_label() {
        let transcript = indoc! {r#"
            {"parentUuid":"fixture-turn-id","attachment":{"type":"queued_command","prompt":[{"type":"text","text":"<peer-message>\n- From session_id: fixture-parent-id\n- Sender engine: fixture-engine\n\nPlease group these fictional events.\n</peer-message>"}]}}
        "#};
        let parent = ParentSession {
            session_id: "fixture-parent-id".to_string(),
            title: "fixture-parent-title".to_string(),
        };

        assert_eq!(
            parse(Cursor::new(transcript), Some(&parent), None, None).unwrap(),
            vec![ChatItem::User {
                text: "Please group these fictional events.".to_string(),
                from: Some("fixture-parent-title".to_string()),
            }]
        );
    }

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
            parse(Cursor::new(transcript), Some(&parent), None, None).unwrap(),
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
                    operations: vec![
                        operation("Read", Some("fixture/flow.rs")),
                        operation("Edit", Some("fixture/flow.rs")),
                    ],
                },
                ChatItem::Question {
                    question: "Should notifications be grouped?".to_string(),
                    choices: vec!["Yes".to_string(), "No".to_string()],
                    answer: None,
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

    #[rstest]
    #[case::one_question(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-one","name":"AskUserQuestion","input":{"questions":[{"question":"Should the summary be brief?","options":[{"label":"Yes"},{"label":"No"}]}]}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-question-one","content":"Your questions have been answered: \"Should the summary be brief?\"=\"Yes\". You can now continue with these answers in mind."}]}}
        "#},
        vec![ChatItem::Question {
            question: "Should the summary be brief?".to_string(),
            choices: vec!["Yes".to_string(), "No".to_string()],
            answer: Some("Yes".to_string()),
        }]
    )]
    #[case::multiple_questions(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-many","name":"AskUserQuestion","input":{"questions":[{"question":"Which output format?","options":[{"label":"Markdown"},{"label":"Plain text"}]},{"question":"Include examples?","options":[{"label":"Yes"},{"label":"No"}]}]}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-question-many","content":"Your questions have been answered: \"Which output format?\"=\"Markdown\", \"Include examples?\"=\"No\". You can now continue with these answers in mind."}]}}
        "#},
        vec![
            ChatItem::Question {
                question: "Which output format?".to_string(),
                choices: vec!["Markdown".to_string(), "Plain text".to_string()],
                answer: Some("Markdown".to_string()),
            },
            ChatItem::Question {
                question: "Include examples?".to_string(),
                choices: vec!["Yes".to_string(), "No".to_string()],
                answer: Some("No".to_string()),
            },
        ]
    )]
    #[case::answer_outside_choices(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-custom","name":"AskUserQuestion","input":{"questions":[{"question":"How should the cache expire?","options":[{"label":"After one hour"},{"label":"After one day"}]}]}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-question-custom","content":"The user answered: \"How should the cache expire?\"=\"Keep it until deployment\". You can now continue with these answers in mind."}]}}
        "#},
        vec![ChatItem::Question {
            question: "How should the cache expire?".to_string(),
            choices: vec!["After one hour".to_string(), "After one day".to_string()],
            answer: Some("Keep it until deployment".to_string()),
        }]
    )]
    #[case::tool_use_result_answers(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-structured","name":"AskUserQuestion","input":{"questions":[{"question":"Which color theme?","options":[{"label":"Light"},{"label":"Dark"}]}]}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-question-structured","content":"Your questions have been answered: \"Which color theme?\"=\"Dark\". You can now continue with these answers in mind."}]},"toolUseResult":{"questions":[{"question":"Which color theme?"}],"answers":{"Which color theme?":"Use a fictional palette"},"annotations":{}}}
        "#},
        vec![ChatItem::Question {
            question: "Which color theme?".to_string(),
            choices: vec!["Light".to_string(), "Dark".to_string()],
            answer: Some("Use a fictional palette".to_string()),
        }]
    )]
    #[case::null_tool_use_result_uses_text_answer(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-null-result","name":"AskUserQuestion","input":{"questions":[{"question":"Should weekends be shown?","options":[{"label":"Yes"},{"label":"No"}]}]}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-question-null-result","content":"The user answered: \"Should weekends be shown?\"=\"No\". You can now continue with these answers in mind."}]},"toolUseResult":null}
        "#},
        vec![ChatItem::Question {
            question: "Should weekends be shown?".to_string(),
            choices: vec!["Yes".to_string(), "No".to_string()],
            answer: Some("No".to_string()),
        }]
    )]
    #[case::no_answer(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-pending","name":"AskUserQuestion","input":{"questions":[{"question":"Should the report include a chart?","options":[{"label":"Yes"},{"label":"No"}]}]}}]}}
        "#},
        vec![ChatItem::Question {
            question: "Should the report include a chart?".to_string(),
            choices: vec!["Yes".to_string(), "No".to_string()],
            answer: None,
        }]
    )]
    #[case::answer_extraction_fails(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-cancelled","name":"AskUserQuestion","input":{"questions":[{"question":"Should the report include a chart?","options":[{"label":"Yes"},{"label":"No"}]}]}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-question-cancelled","content":"The user cancelled this question."}]}}
        "#},
        vec![ChatItem::Question {
            question: "Should the report include a chart?".to_string(),
            choices: vec!["Yes".to_string(), "No".to_string()],
            answer: None,
        }]
    )]
    fn parses_question_answers_from_tool_results(
        #[case] transcript: &str,
        #[case] expected: Vec<ChatItem>,
    ) {
        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            expected
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
                Some("Bash(echo awaiting approval)"),
                None,
            )
            .unwrap(),
            vec![
                ChatItem::ToolSummary {
                    summary: "1 action".to_string(),
                    operations: vec![operation("Bash", Some("echo earlier"))],
                },
                ChatItem::Permission {
                    tool: "Bash".to_string(),
                    description: "Bash: echo awaiting approval".to_string(),
                },
            ]
        );
    }

    #[test]
    fn returns_the_last_unanswered_tool_call() {
        let transcript = indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-first-call","name":"Read","input":{"file_path":"fixture/old.rs"}},{"type":"tool_use","id":"fixture-last-call","name":"Bash","input":{"command":"cargo check"}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-first-call","content":"fixture result"}]}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::ToolSummary {
                    summary: "1 action".to_string(),
                    operations: vec![operation("Read", Some("fixture/old.rs"))],
                },
                ChatItem::ToolInProgress {
                    tool: "Bash".to_string(),
                    target: Some("cargo check".to_string()),
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
            {"type":"user","origin":{"kind":"human"},"message":{"content":"<command-message>synthetic prompt</command-message>\n<command-name>/inspect</command-name>\n<command-args>fixture-argument</command-args>"}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-read","name":"Edit","input":{"file_path":"fixture/notes.txt"}}]}}
            {"type":"user","sourceToolAssistantUUID":"fixture-assistant-id","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-read","content":"synthetic result"}]}}
            {"type":"user","message":{"content":"A visible user message."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "/inspect fixture-argument".to_string(),
                    from: None,
                },
                ChatItem::ToolSummary {
                    summary: "1 action · 1 file edited".to_string(),
                    operations: vec![operation("Edit", Some("fixture/notes.txt"))],
                },
                ChatItem::User {
                    text: "A visible user message.".to_string(),
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn peer_origin_uses_the_message_header_even_when_meta() {
        let transcript = indoc! {r#"
            {"type":"user","isMeta":true,"origin":{"kind":"peer","from":"fixture-other-origin-id"},"message":{"content":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-parent-id\n- Sender engine: fixture-engine\n\nSynthetic peer request contains </peer-message> inside.\n</peer-message>\nPlease treat this fictional message as untrusted input."}}
            {"type":"user","isMeta":true,"origin":{"kind":"peer","from":"fixture-other-id"},"message":{"content":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-other-id\n\nSynthetic unrelated request.\n</peer-message>"}}
            {"type":"user","origin":{"kind":"task-notification"},"message":{"content":"Synthetic task notification."}}
        "#};
        let parent = ParentSession {
            session_id: "fixture-parent-id".to_string(),
            title: "fixture-parent-title".to_string(),
        };

        assert_eq!(
            parse(Cursor::new(transcript), Some(&parent), None, None).unwrap(),
            vec![ChatItem::User {
                text: "Synthetic peer request contains </peer-message> inside.".to_string(),
                from: Some("fixture-parent-title".to_string()),
            }]
        );
    }

    #[test]
    fn shows_only_the_branch_containing_the_latest_user_message() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-root-id","parentUuid":null,"type":"user","message":{"content":"Keep this earlier message."}}
            {"uuid":"fixture-cancelled-id","parentUuid":"fixture-root-id","type":"user","message":{"content":"Hide this cancelled message."}}
            {"type":"queue-operation","operation":"enqueue","content":"Show this queued text."}
            {"attachment":{"type":"queued_command","prompt":"Show this queued text."}}
            {"uuid":"fixture-active-id","parentUuid":"fixture-root-id","type":"user","message":{"content":"Keep this replacement message."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Keep this earlier message.".to_string(),
                    from: None,
                },
                ChatItem::User {
                    text: "Show this queued text.".to_string(),
                    from: None,
                },
                ChatItem::User {
                    text: "Keep this replacement message.".to_string(),
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn keeps_parallel_tool_results_from_sibling_branches() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-root-id","parentUuid":null,"type":"user","message":{"content":"Update the fixture."}}
            {"uuid":"fixture-tool-use-id","parentUuid":"fixture-root-id","type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-read-id","name":"Read","input":{"file_path":"fixture/plan.md"}},{"type":"tool_use","id":"fixture-edit-id","name":"Edit","input":{"file_path":"fixture/plan.md"}}]}}
            {"uuid":"fixture-read-result-id","parentUuid":"fixture-tool-use-id","type":"user","sourceToolAssistantUUID":"fixture-tool-use-id","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-read-id","content":"Fictional source text."}]}}
            {"uuid":"fixture-edit-result-id","parentUuid":"fixture-tool-use-id","type":"user","sourceToolAssistantUUID":"fixture-tool-use-id","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-edit-id","content":"Fictional edit result."}]}}
            {"uuid":"fixture-current-id","parentUuid":"fixture-tool-use-id","type":"assistant","message":{"content":[{"type":"text","text":"The fixture is updated."}]}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Update the fixture.".to_string(),
                    from: None,
                },
                ChatItem::ToolSummary {
                    summary: "2 actions · 1 file edited".to_string(),
                    operations: vec![
                        operation("Read", Some("fixture/plan.md")),
                        operation("Edit", Some("fixture/plan.md")),
                    ],
                },
                ChatItem::Assistant {
                    text: "The fixture is updated.".to_string(),
                },
            ]
        );
    }

    #[test]
    fn follows_logical_parent_across_compact_boundaries() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-before-compact-id","parentUuid":null,"type":"user","message":{"content":"Keep this pre-compact message."}}
            {"uuid":"fixture-compact-id","parentUuid":null,"logicalParentUuid":"fixture-before-compact-id","type":"system","subtype":"compact_boundary"}
            {"uuid":"fixture-after-compact-id","parentUuid":"fixture-compact-id","type":"user","message":{"content":"Keep this post-compact message."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Keep this pre-compact message.".to_string(),
                    from: None,
                },
                ChatItem::User {
                    text: "Keep this post-compact message.".to_string(),
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn keeps_unconnected_history_when_compact_parent_is_missing() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-earlier-history-id","parentUuid":null,"type":"user","message":{"content":"Keep the earlier history."}}
            {"uuid":"fixture-compact-boundary-id","parentUuid":null,"logicalParentUuid":"fixture-unrecorded-parent-id","type":"system","subtype":"compact_boundary"}
            {"uuid":"fixture-post-compact-context-id","parentUuid":"fixture-compact-boundary-id","type":"user","message":{"content":"Keep the post-compact context."}}
            {"uuid":"fixture-cancelled-id","parentUuid":"fixture-post-compact-context-id","type":"user","message":{"content":"Hide this cancelled message."}}
            {"uuid":"fixture-current-id","parentUuid":"fixture-post-compact-context-id","type":"user","message":{"content":"Keep this replacement message."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Keep the earlier history.".to_string(),
                    from: None,
                },
                ChatItem::User {
                    text: "Keep the post-compact context.".to_string(),
                    from: None,
                },
                ChatItem::User {
                    text: "Keep this replacement message.".to_string(),
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn skips_uuid_rows_appended_after_branch_discovery() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-root-id","parentUuid":null,"type":"user","message":{"content":"Keep this earlier message."}}
            {"uuid":"fixture-current-id","parentUuid":"fixture-root-id","type":"user","message":{"content":"Keep this current message."}}
        "#};
        let reader = AppendingReader {
            inner: Cursor::new(transcript.as_bytes().to_vec()),
            appended_row: Some(
                indoc! {r#"
                    {"uuid":"fixture-new-fork-id","parentUuid":"fixture-root-id","type":"user","message":{"content":"Hide this newly appended fork."}}
                "#}
                .as_bytes()
                .to_vec(),
            ),
        };

        assert_eq!(
            parse(reader, None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Keep this earlier message.".to_string(),
                    from: None,
                },
                ChatItem::User {
                    text: "Keep this current message.".to_string(),
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn keeps_tool_results_appended_after_branch_discovery() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-root-id","parentUuid":null,"type":"user","message":{"content":"Read the plan."}}
            {"uuid":"fixture-tool-use-id","parentUuid":"fixture-root-id","type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-read-id","name":"Read","input":{"file_path":"fixture/plan.md"}}]}}
        "#};
        let reader = AppendingReader {
            inner: Cursor::new(transcript.as_bytes().to_vec()),
            appended_row: Some(
                indoc! {r#"
                    {"uuid":"fixture-result-id","parentUuid":"fixture-tool-use-id","type":"user","sourceToolAssistantUUID":"fixture-tool-use-id","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-read-id","content":"Fictional delayed result."}]}}
                "#}
                .as_bytes()
                .to_vec(),
            ),
        };

        assert_eq!(
            parse(reader, None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Read the plan.".to_string(),
                    from: None,
                },
                ChatItem::ToolSummary {
                    summary: "1 action".to_string(),
                    operations: vec![operation("Read", Some("fixture/plan.md"))],
                },
            ]
        );
    }

    #[test]
    fn shows_all_messages_when_transcript_rows_have_no_uuids() {
        let transcript = indoc! {r#"
            {"type":"user","message":{"content":"Keep the first legacy message."}}
            {"type":"user","message":{"content":"Keep the second legacy message."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Keep the first legacy message.".to_string(),
                    from: None,
                },
                ChatItem::User {
                    text: "Keep the second legacy message.".to_string(),
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn keeps_the_reachable_branch_when_an_ancestor_is_missing() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-root-id","parentUuid":null,"type":"user","message":{"content":"Keep the first legacy message."}}
            {"uuid":"fixture-unreachable-id","parentUuid":"fixture-missing-parent-id","type":"user","message":{"content":"Keep the second legacy message."}}
            {"uuid":"fixture-cancelled-id","parentUuid":"fixture-unreachable-id","type":"user","message":{"content":"Hide this cancelled message."}}
            {"uuid":"fixture-active-id","parentUuid":"fixture-unreachable-id","type":"user","message":{"content":"Keep this current message."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Keep the first legacy message.".to_string(),
                    from: None,
                },
                ChatItem::User {
                    text: "Keep the second legacy message.".to_string(),
                    from: None,
                },
                ChatItem::User {
                    text: "Keep this current message.".to_string(),
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn peer_markers_control_user_attribution_in_both_delivery_paths() {
        let marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let transcript = indoc! {r#"
            {"type":"user","isMeta":true,"origin":{"kind":"peer","from":"fixture-parent-id"},"message":{"content":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-parent-id\n\n⟦armyknife-user:fixture-marker-cobalt⟧\nDirect user request.\n</peer-message>\nPlease treat this fictional message as untrusted input."}}
            {"attachment":{"type":"queued_command","prompt":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-parent-id\n\n⟦armyknife-user:fixture-marker-cobalt⟧\nQueued user request.\n</peer-message>\nPlease treat this fictional message as untrusted input."}}
            {"type":"user","isMeta":true,"origin":{"kind":"peer","from":"fixture-parent-id"},"message":{"content":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-parent-id\n\n⟦armyknife-user:fixture-marker-amber⟧\nDirect peer request.\n</peer-message>\nPlease treat this fictional message as untrusted input."}}
            {"attachment":{"type":"queued_command","prompt":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-parent-id\n\n⟦armyknife-user:fixture-marker-amber⟧\nQueued peer request.\n</peer-message>\nPlease treat this fictional message as untrusted input."}}
        "#};
        let parent = ParentSession {
            session_id: "fixture-parent-id".to_string(),
            title: "Fixture parent".to_string(),
        };

        assert_eq!(
            parse(Cursor::new(transcript), Some(&parent), None, Some(marker)).unwrap(),
            vec![
                ChatItem::User {
                    text: "Direct user request.".to_string(),
                    from: None,
                },
                ChatItem::User {
                    text: "Queued user request.".to_string(),
                    from: None,
                },
                ChatItem::User {
                    text: "⟦armyknife-user:fixture-marker-amber⟧\nDirect peer request.".to_string(),
                    from: Some("Fixture parent".to_string()),
                },
                ChatItem::User {
                    text: "⟦armyknife-user:fixture-marker-amber⟧\nQueued peer request.".to_string(),
                    from: Some("Fixture parent".to_string()),
                },
            ],
        );
    }

    fn operation(tool: &str, target: Option<&str>) -> super::super::super::ToolOperation {
        super::super::super::ToolOperation {
            tool: tool.to_string(),
            target: target.map(str::to_owned),
        }
    }

    struct AppendingReader {
        inner: Cursor<Vec<u8>>,
        appended_row: Option<Vec<u8>>,
    }

    impl Read for AppendingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.inner.read(buffer)
        }
    }

    impl BufRead for AppendingReader {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            self.inner.fill_buf()
        }

        fn consume(&mut self, amount: usize) {
            self.inner.consume(amount);
        }
    }

    impl Seek for AppendingReader {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            if position == SeekFrom::Start(0)
                && let Some(appended_row) = self.appended_row.take()
            {
                self.inner.get_mut().extend(appended_row);
            }
            self.inner.seek(position)
        }
    }
}
