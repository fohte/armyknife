use serde_json::Value;

use crate::commands::agent::watch_web::messages::UserContentPart;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum UserOrigin {
    Human,
    Peer,
    Legacy,
}

pub(super) fn is_tool_result_only_user_record(record: &Value) -> bool {
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

pub(super) fn user_origin(record: &Value, message: &Value) -> Option<UserOrigin> {
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

pub(super) fn slash_command_text(text: &str) -> Option<String> {
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

pub(super) fn user_text(message: &Value) -> String {
    content_text(message.get("content"))
}

pub(super) fn user_content(content: Option<&Value>) -> (String, Vec<UserContentPart>) {
    let mut images = Vec::new();
    if let Some(Value::Array(blocks)) = content {
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("image") {
                continue;
            }
            let Some(source) = block.get("source") else {
                continue;
            };
            if source.get("type").and_then(Value::as_str) != Some("base64") {
                continue;
            }
            let Some(media_type) = source.get("media_type").and_then(Value::as_str) else {
                continue;
            };
            if !matches!(
                media_type,
                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
            ) {
                continue;
            }
            let Some(data) = source.get("data").and_then(Value::as_str) else {
                continue;
            };
            if data.is_empty()
                || !data.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=')
                })
            {
                continue;
            }
            images.push(UserContentPart::Image {
                source: format!("data:{media_type};base64,{data}"),
                available: true,
            });
        }
    }
    (content_text(content), images)
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

pub(super) fn content_blocks(content: &Value) -> Vec<&Value> {
    match content {
        Value::Array(blocks) => blocks.iter().collect(),
        Value::String(_) => Vec::new(),
        _ => Vec::new(),
    }
}
