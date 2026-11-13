use std::collections::HashMap;

use chrono::{DateTime, FixedOffset, Utc};

use crate::commands::agent::store::AgentFile;

use super::super::file_attachments;
use super::ChatItem;

pub(super) fn merge_stored_files(
    session_id: &str,
    files: &[AgentFile],
    messages: &mut Vec<ChatItem>,
) {
    let mut groups: Vec<(Option<String>, DateTime<Utc>, Vec<AgentFile>)> = Vec::new();
    let mut group_indices = HashMap::new();
    for file in files {
        let key = (file.caption.clone(), file.created_at);
        let index = match group_indices.get(&key) {
            Some(index) => *index,
            None => {
                let index = groups.len();
                groups.push((file.caption.clone(), file.created_at, Vec::new()));
                group_indices.insert(key, index);
                index
            }
        };
        groups[index].2.push(file.clone());
    }
    let mut attachments = groups
        .into_iter()
        .map(|(caption, created_at, grouped_files)| {
            (
                created_at,
                ChatItem::FileAttachment {
                    files: file_attachments::stored_chat_files(session_id, &grouped_files),
                    caption,
                    timestamp: Some(created_at.to_rfc3339()),
                },
            )
        })
        .collect::<Vec<_>>();
    attachments.sort_by_key(|(created_at, _)| *created_at);

    let original_messages = std::mem::take(messages);
    let mut merged = Vec::with_capacity(original_messages.len() + attachments.len());
    let mut attachments = attachments.into_iter().peekable();
    for message in original_messages {
        if let Some(timestamp) = chat_item_timestamp(&message) {
            while attachments
                .peek()
                .is_some_and(|(created_at, _)| timestamp > *created_at)
            {
                if let Some((_, attachment)) = attachments.next() {
                    merged.push(attachment);
                }
            }
        }
        merged.push(message);
    }
    merged.extend(attachments.map(|(_, attachment)| attachment));
    *messages = merged;
}

fn chat_item_timestamp(message: &ChatItem) -> Option<DateTime<FixedOffset>> {
    let timestamp = match message {
        ChatItem::User { timestamp, .. }
        | ChatItem::QueuedUser { timestamp, .. }
        | ChatItem::UserWithContent { timestamp, .. }
        | ChatItem::Peer { timestamp, .. }
        | ChatItem::Event { timestamp, .. }
        | ChatItem::FileAttachment { timestamp, .. } => timestamp.as_deref(),
        ChatItem::Decision { created_at, .. } => Some(created_at.as_str()),
        _ => None,
    }?;
    DateTime::parse_from_rfc3339(timestamp).ok()
}
