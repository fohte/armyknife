use std::collections::{HashMap, HashSet};

use crate::commands::agent::store::{WatchWebQueuedAttachment, WatchWebQueuedPrompt};

use super::{ChatItem, UserContentPart};

const ID_MARKER_PREFIX: &str = "<!-- armyknife-queued-prompt-id:";
const ID_MARKER_SUFFIX: &str = " -->";

pub(in crate::commands::agent::watch_web) fn add_id_marker(
    content: &str,
    message_id: &str,
) -> String {
    let marker = format!("{ID_MARKER_PREFIX}{message_id}{ID_MARKER_SUFFIX}");
    if content.is_empty() {
        marker
    } else {
        format!("{content}\n{marker}")
    }
}

pub(in crate::commands::agent::watch_web) fn id_from_content(content: &str) -> Option<&str> {
    let content = content.trim_end();
    let marker_start = content.rfind(ID_MARKER_PREFIX)?;
    let marker = &content[marker_start..];
    let message_id = marker
        .strip_prefix(ID_MARKER_PREFIX)?
        .strip_suffix(ID_MARKER_SUFFIX)?;
    (!message_id.is_empty()).then_some(message_id)
}

fn remove_id_marker(content: &mut String) -> Option<String> {
    let marker_start = content.rfind(ID_MARKER_PREFIX)?;
    let marker = content[marker_start..].to_owned();
    let message_id = id_from_content(&marker)?.to_owned();
    let visible_content = without_marker_separator(&content[..marker_start]).to_owned();
    *content = visible_content;
    Some(message_id)
}

pub(super) fn strip_id_marker(content: &str) -> String {
    let Some(marker_start) = content.rfind(ID_MARKER_PREFIX) else {
        return content.to_owned();
    };
    let marker = &content[marker_start..];
    if id_from_content(marker).is_none() {
        return content.to_owned();
    }
    without_marker_separator(&content[..marker_start]).to_owned()
}

fn without_marker_separator(content: &str) -> &str {
    content.strip_suffix('\n').unwrap_or(content)
}

pub(super) fn user_content(prompt: &WatchWebQueuedPrompt) -> Vec<UserContentPart> {
    let mut content = Vec::with_capacity(prompt.attachments.len() + 1);
    if !prompt.content.is_empty() {
        content.push(UserContentPart::Text {
            text: prompt.content.clone(),
        });
    }
    content.extend(
        prompt
            .attachments
            .iter()
            .map(|attachment| match attachment {
                WatchWebQueuedAttachment::Image { source } => UserContentPart::Image {
                    source: source.clone(),
                    available: false,
                },
                WatchWebQueuedAttachment::File { name } => {
                    UserContentPart::File { name: name.clone() }
                }
            }),
    );
    content
}

pub(super) fn resolve_transcript_messages(
    messages: &mut [ChatItem],
    queued_prompts: &[WatchWebQueuedPrompt],
) -> Vec<i64> {
    let prompts_by_message_id = queued_prompts
        .iter()
        .filter_map(|prompt| {
            prompt
                .message_id
                .as_deref()
                .map(|message_id| (message_id, prompt))
        })
        .collect::<HashMap<_, _>>();
    let mut resolved = Vec::new();
    for message in messages {
        let message_id = match message {
            ChatItem::User { text, .. } => remove_id_marker(text),
            ChatItem::UserWithContent { content, .. } => {
                content.iter_mut().find_map(|part| match part {
                    UserContentPart::Text { text } => remove_id_marker(text),
                    UserContentPart::Image { .. } | UserContentPart::File { .. } => None,
                })
            }
            _ => None,
        };
        let Some(message_id) = message_id else {
            continue;
        };
        let Some(prompt) = prompts_by_message_id.get(message_id.as_str()).copied() else {
            continue;
        };
        let (timestamp, from, rewind_id) = match message {
            ChatItem::User {
                timestamp,
                from,
                rewind_id,
                ..
            }
            | ChatItem::UserWithContent {
                timestamp,
                from,
                rewind_id,
                ..
            } => (timestamp.clone(), from.clone(), rewind_id.clone()),
            _ => continue,
        };
        let content = user_content(prompt);
        *message = if prompt.attachments.is_empty() {
            ChatItem::User {
                text: prompt.content.clone(),
                from,
                timestamp,
                rewind_id,
                queued_prompt_id: prompt.message_id.clone(),
            }
        } else {
            ChatItem::UserWithContent {
                content,
                from,
                timestamp,
                rewind_id,
                queued_prompt_id: prompt.message_id.clone(),
            }
        };
        if !prompt.resolved {
            resolved.push(prompt.id);
        }
    }
    resolved
}

pub(super) fn append_unresolved_prompts(
    messages: &mut Vec<ChatItem>,
    queued_prompts: &[WatchWebQueuedPrompt],
    resolved_prompt_ids: &[i64],
) {
    let resolved_prompt_ids = resolved_prompt_ids.iter().copied().collect::<HashSet<_>>();
    messages.extend(
        queued_prompts
            .iter()
            .filter(|prompt| !prompt.resolved && !resolved_prompt_ids.contains(&prompt.id))
            .map(|prompt| ChatItem::QueuedUser {
                content: user_content(prompt),
                from: None,
                timestamp: Some(prompt.sent_at.to_rfc3339()),
            }),
    );
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{add_id_marker, id_from_content, strip_id_marker};

    #[rstest]
    #[case::empty("")]
    #[case::without_trailing_newline("A fixture request.")]
    #[case::with_trailing_newline("A fixture request.\n")]
    fn marker_round_trip_preserves_visible_content(#[case] content: &str) {
        let marked = add_id_marker(content, "fixture-message-cyan");

        assert_eq!(
            (id_from_content(&marked), strip_id_marker(&marked)),
            (Some("fixture-message-cyan"), content.to_owned()),
        );
    }
}
