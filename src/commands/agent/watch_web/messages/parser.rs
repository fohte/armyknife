pub(super) mod cards;
pub(super) mod classifier;
pub(super) mod claude;
pub(super) mod codex;
pub(super) mod jsonl;
pub(super) mod queue;
pub(super) mod skill_suggestions;
pub(super) mod tool_summary;

use tool_summary::{
    CodexToolSummaryRecord, ToolSummary, codex_edited_file_paths, codex_tool_summary_record,
    operation_target, tool_target,
};

use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{ChatItem, ParentSession, UserContentPart};

pub(super) fn record_timestamp(record: &Value) -> Option<DateTime<Utc>> {
    record
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

pub(super) fn append_text(
    items: &mut Vec<ChatItem>,
    text: String,
    is_user: bool,
    parent: Option<&ParentSession>,
    timestamp: Option<DateTime<Utc>>,
) {
    let image_directory = super::super::image_attachments::storage_directory().ok();
    append_text_with_image_directory(
        items,
        text,
        is_user,
        parent,
        image_directory.as_deref(),
        timestamp,
    );
}

fn append_text_with_image_directory(
    items: &mut Vec<ChatItem>,
    text: String,
    is_user: bool,
    parent: Option<&ParentSession>,
    image_directory: Option<&Path>,
    timestamp: Option<DateTime<Utc>>,
) {
    if text.trim().is_empty() {
        return;
    }
    if is_user {
        append_text_with_user_marker_and_image_directory(
            items,
            text,
            parent,
            None,
            Vec::new(),
            image_directory,
            timestamp,
        );
    } else {
        items.push(ChatItem::Assistant { text });
    }
}

fn append_text_with_user_marker_and_image_directory(
    items: &mut Vec<ChatItem>,
    text: String,
    parent: Option<&ParentSession>,
    marker: Option<&str>,
    images: Vec<UserContentPart>,
    image_directory: Option<&Path>,
    timestamp: Option<DateTime<Utc>>,
) {
    let classification =
        classifier::classify_inbound(&serde_json::json!({"type":"user"}), &text, marker);
    append_classification(
        items,
        classification,
        images,
        image_directory,
        parent,
        timestamp,
        None,
    );
}

pub(in crate::commands::agent::watch_web::messages::parser) fn append_user_text_with_rewind_id(
    items: &mut Vec<ChatItem>,
    text: String,
    is_initial_user_message: bool,
    parent: Option<&ParentSession>,
    timestamp: Option<DateTime<Utc>>,
    rewind_id: Option<String>,
) {
    if text.trim().is_empty() {
        return;
    }
    let context_free_user_text = if is_initial_user_message
        && crate::commands::agent::peer_message::extract_peer_message(&text, None).is_none()
    {
        Some(classifier::user_text_without_session_context(&text))
    } else {
        None
    };
    let classification = match context_free_user_text {
        Some(context_free_user_text) if context_free_user_text != text => {
            classifier::MessageKind::User {
                text: context_free_user_text,
            }
        }
        _ => classifier::classify_codex_inbound(&serde_json::json!({"type":"user"}), &text),
    };
    append_classification(
        items,
        classification,
        Vec::new(),
        None,
        parent,
        timestamp,
        rewind_id,
    );
}

fn append_classification(
    items: &mut Vec<ChatItem>,
    classification: classifier::MessageKind,
    images: Vec<UserContentPart>,
    image_directory: Option<&Path>,
    parent: Option<&ParentSession>,
    timestamp: Option<DateTime<Utc>>,
    rewind_id: Option<String>,
) {
    match classification {
        classifier::MessageKind::User { text } if !text.trim().is_empty() || !images.is_empty() => {
            items.push(user_item_with_image_directory(
                text,
                None,
                images,
                image_directory,
                timestamp,
                rewind_id,
            ));
        }
        classifier::MessageKind::PeerIn { session_id, text }
            if !text.trim().is_empty() || !images.is_empty() =>
        {
            let session_title = parent
                .filter(|parent| parent.session_id == session_id)
                .map(|parent| parent.title.clone());
            items.push(peer_item(
                session_id,
                super::PeerDirection::From,
                session_title,
                text,
                images,
                image_directory,
                timestamp,
            ));
        }
        classifier::MessageKind::SubagentReport {
            agent_id,
            agent_name,
            text,
        } if !text.trim().is_empty() => {
            items.push(ChatItem::SubagentReport {
                agent_id,
                agent_name,
                description: None,
                text,
            });
        }
        classifier::MessageKind::Event { kind, text, failed } => {
            if !text.trim().is_empty() {
                let kind = match kind {
                    classifier::EventKind::Armyknife => super::EventKind::Armyknife,
                    classifier::EventKind::CompactBoundary => super::EventKind::CompactBoundary,
                    classifier::EventKind::System => super::EventKind::System,
                    classifier::EventKind::Hook => super::EventKind::Hook,
                };
                items.push(ChatItem::Event {
                    kind,
                    text,
                    failed,
                    timestamp: if kind == super::EventKind::Hook {
                        None
                    } else {
                        timestamp.map(|timestamp| timestamp.to_rfc3339())
                    },
                });
            }
        }
        classifier::MessageKind::User { .. }
        | classifier::MessageKind::PeerIn { .. }
        | classifier::MessageKind::SubagentReport { .. }
        | classifier::MessageKind::Hidden => {}
    }
}

pub(in crate::commands::agent::watch_web::messages::parser) fn append_classification_with_rewind_id(
    items: &mut Vec<ChatItem>,
    classification: classifier::MessageKind,
    images: Vec<UserContentPart>,
    image_directory: Option<&Path>,
    parent: Option<&ParentSession>,
    timestamp: Option<DateTime<Utc>>,
    rewind_id: Option<String>,
) {
    append_classification(
        items,
        classification,
        images,
        image_directory,
        parent,
        timestamp,
        rewind_id,
    );
}

fn peer_item(
    session_id: String,
    direction: super::PeerDirection,
    session_title: Option<String>,
    text: String,
    images: Vec<UserContentPart>,
    image_directory: Option<&Path>,
    timestamp: Option<DateTime<Utc>>,
) -> ChatItem {
    let (text, content) = user_content_parts_with_image_directory(text, images, image_directory);
    let content = content.unwrap_or_else(|| vec![UserContentPart::Text { text }]);
    ChatItem::Peer {
        direction,
        session_id,
        session_title,
        content,
        timestamp: timestamp.map(|timestamp| timestamp.to_rfc3339()),
        failed: false,
    }
}

fn user_item_with_image_directory(
    text: String,
    from: Option<String>,
    images: Vec<UserContentPart>,
    image_directory: Option<&Path>,
    timestamp: Option<DateTime<Utc>>,
    rewind_id: Option<String>,
) -> ChatItem {
    let (text, content) = user_content_parts_with_image_directory(text, images, image_directory);
    match content {
        Some(content) => ChatItem::UserWithContent {
            content,
            rewind_id,
            from,
            timestamp: timestamp.map(|timestamp| timestamp.to_rfc3339()),
            queued_prompt_id: None,
        },
        None => ChatItem::User {
            text,
            rewind_id,
            from,
            timestamp: timestamp.map(|timestamp| timestamp.to_rfc3339()),
            queued_prompt_id: None,
        },
    }
}

fn user_content_parts_with_image_directory(
    text: String,
    images: Vec<UserContentPart>,
    _image_directory: Option<&Path>,
) -> (String, Option<Vec<UserContentPart>>) {
    if images.is_empty() {
        return (text, None);
    }

    let mut content = Vec::new();
    let mut remaining_images = images.into_iter();
    let mut cursor = 0;
    while let Some((start, end)) = image_placeholder(&text[cursor..]) {
        let Some(image) = remaining_images.next() else {
            break;
        };
        let start = cursor + start;
        let end = cursor + end;
        if start > cursor {
            content.push(UserContentPart::Text {
                text: text[cursor..start].to_owned(),
            });
        }
        content.push(image);
        cursor = end;
    }
    if cursor < text.len() {
        content.push(UserContentPart::Text {
            text: text[cursor..].to_owned(),
        });
    }
    content.extend(remaining_images);
    (text, Some(content))
}

fn image_placeholder(text: &str) -> Option<(usize, usize)> {
    let mut search_from = 0;
    while let Some(offset) = text[search_from..].find("[Image #") {
        let start = search_from + offset;
        let number_start = start + "[Image #".len();
        let number_end = text[number_start..].find(']')? + number_start;
        if !text[number_start..number_end].is_empty()
            && text[number_start..number_end]
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            return Some((start, number_end + 1));
        }
        search_from = start + "[Image #".len();
    }
    None
}

#[cfg(test)]
mod tests {
    use indoc::formatdoc;

    use super::{ChatItem, append_text_with_image_directory};
    #[test]
    fn leaves_attachment_path_text_in_unmarked_transcript_messages() {
        let text = indoc::indoc! {"
            Inspect the attachments.

            Attached file(s):
            - /fixture/upload/image.png
            - /fixture/upload/notes.txt
        "}
        .to_string();
        let mut items = Vec::new();

        append_text_with_image_directory(&mut items, text.clone(), true, None, None, None);

        assert_eq!(
            items,
            vec![ChatItem::User {
                rewind_id: None,
                text,
                from: None,
                timestamp: None,
                queued_prompt_id: None,
            }]
        );
    }

    #[test]
    fn retains_the_attachment_listing_when_storage_directory_resolution_fails() {
        let text = formatdoc! {"
            Inspect this image.

            Attached image(s):
            - /unavailable/00000000-0000-4000-8000-000000000001.png
        "};
        let mut items = Vec::new();

        append_text_with_image_directory(&mut items, text.clone(), true, None, None, None);

        assert_eq!(
            items,
            vec![ChatItem::User {
                rewind_id: None,
                text,
                from: None,
                timestamp: None,
                queued_prompt_id: None,
            }],
        );
    }

    #[test]
    fn preserves_notice_text_quoted_by_the_user_before_the_transport_notice() {
        let marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let notice = "This came from another Claude session — this fictional notice is retained.";
        let message = formatdoc! {"
            {marker}
            This message was typed by the human user in armyknife watch-web.
            The user quoted this notice:

            {notice}

            {notice}"
        };
        let expected = formatdoc! {"
            The user quoted this notice:

            {notice}"
        };

        assert_eq!(
            super::classifier::classify_inbound(
                &serde_json::json!({"type":"user"}),
                &message,
                Some(marker),
            ),
            super::classifier::MessageKind::User { text: expected },
        );
    }
}
