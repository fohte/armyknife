use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;

use crate::commands::agent::watch_web::{
    image_attachments::{MAX_IMAGE_BYTES, MAX_IMAGES_PER_MESSAGE},
    messages::{UserContentPart, claude_content_text as content_text},
};

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
    user_content_with_limits(content, MAX_IMAGE_BYTES, MAX_IMAGES_PER_MESSAGE)
}

fn user_content_with_limits(
    content: Option<&Value>,
    max_image_bytes: usize,
    max_images: usize,
) -> (String, Vec<UserContentPart>) {
    let mut images = Vec::new();
    let mut total_image_bytes = 0_usize;
    let mut total_encoded_bytes = 0_usize;
    let mut seen_images = 0_usize;
    let max_encoded_image_bytes = max_image_bytes.div_ceil(3).saturating_mul(4);
    let max_encoded_total_bytes =
        max_encoded_image_bytes.saturating_add(max_images.saturating_mul(4));
    if let Some(Value::Array(blocks)) = content {
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("image") {
                continue;
            }
            if seen_images >= max_images {
                continue;
            }
            seen_images += 1;
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
            if data.is_empty() || data.len() > max_encoded_image_bytes {
                continue;
            }
            let Some(next_encoded_bytes) = total_encoded_bytes
                .checked_add(data.len())
                .filter(|size| *size <= max_encoded_total_bytes)
            else {
                continue;
            };
            let Ok(decoded) = STANDARD.decode(data) else {
                continue;
            };
            let image_bytes = decoded.len();
            drop(decoded);
            let Some(next_image_bytes) = total_image_bytes
                .checked_add(image_bytes)
                .filter(|size| *size <= max_image_bytes)
            else {
                continue;
            };
            total_encoded_bytes = next_encoded_bytes;
            total_image_bytes = next_image_bytes;
            images.push(UserContentPart::Image {
                source: format!("data:{media_type};base64,{data}"),
                available: true,
            });
        }
    }
    (content_text(content), images)
}

pub(super) fn content_blocks(content: &Value) -> Vec<&Value> {
    match content {
        Value::Array(blocks) => blocks.iter().collect(),
        Value::String(_) => Vec::new(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use indoc::indoc;
    use rstest::rstest;
    use serde_json::{Value, json};

    use super::{user_content, user_content_with_limits};
    use crate::commands::agent::watch_web::messages::{ChatItem, UserContentPart};

    #[test]
    fn preserves_images_in_user_messages_and_queued_prompts() {
        let transcript = indoc! {r#"
            {"type":"user","origin":{"kind":"human"},"message":{"content":[{"type":"text","text":"User [Image #1] request."},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}}]}}
            {"attachment":{"type":"queued_command","prompt":[{"type":"text","text":"Queued [Image #1] prompt."},{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"AQID"}}]}}
        "#};

        assert_eq!(
            super::super::parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::UserWithContent {
                    content: vec![
                        UserContentPart::Text {
                            text: "User ".to_string(),
                        },
                        UserContentPart::Image {
                            source: "data:image/png;base64,AAAA".to_string(),
                            available: true,
                        },
                        UserContentPart::Text {
                            text: " request.".to_string(),
                        },
                    ],
                    from: None,
                },
                ChatItem::UserWithContent {
                    content: vec![
                        UserContentPart::Text {
                            text: "Queued ".to_string(),
                        },
                        UserContentPart::Image {
                            source: "data:image/jpeg;base64,AQID".to_string(),
                            available: true,
                        },
                        UserContentPart::Text {
                            text: " prompt.".to_string(),
                        },
                    ],
                    from: None,
                },
            ],
        );
    }

    #[test]
    fn ignores_an_image_over_the_per_image_limit_without_adding_its_data_to_text() {
        let content = json!([
            {"type": "text", "text": "Keep this prompt."},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAAAAA=="}}
        ]);

        assert_eq!(
            user_content_with_limits(Some(&content), 3, 16),
            ("Keep this prompt.".to_string(), Vec::new()),
        );
    }

    #[test]
    fn ignores_images_that_exceed_the_aggregate_limit_without_growing_text() {
        let content = json!([
            {"type": "text", "text": "Keep this prompt."},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAAA"}},
            {"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": "AQID"}}
        ]);

        assert_eq!(
            user_content_with_limits(Some(&content), 5, 16),
            (
                "Keep this prompt.".to_string(),
                vec![UserContentPart::Image {
                    source: "data:image/png;base64,AAAA".to_string(),
                    available: true,
                }],
            ),
        );
    }

    #[test]
    fn limits_the_number_of_imported_images() {
        let mut blocks = vec![json!({"type": "text", "text": "Keep this prompt."})];
        blocks.extend((0..3).map(|_| {
            json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAAA"}})
        }));
        let content = Value::Array(blocks);
        let image = UserContentPart::Image {
            source: "data:image/png;base64,AAAA".to_string(),
            available: true,
        };

        assert_eq!(
            user_content_with_limits(Some(&content), 10, 2),
            ("Keep this prompt.".to_string(), vec![image; 2]),
        );
    }

    #[rstest]
    #[case::invalid_alphabet("A!AA")]
    #[case::invalid_padding("AAAA=")]
    fn keeps_malformed_image_data_out_of_the_message(#[case] data: &str) {
        let content = json!([
            {"type": "text", "text": "Keep this prompt."},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": data}}
        ]);

        assert_eq!(
            user_content(Some(&content)),
            ("Keep this prompt.".to_string(), Vec::new()),
        );
    }
}
