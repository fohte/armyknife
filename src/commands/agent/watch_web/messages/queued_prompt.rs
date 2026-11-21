use std::collections::{HashMap, HashSet};

use crate::commands::agent::store::{WatchWebQueuedAttachment, WatchWebQueuedPrompt};

use super::action_event;
use super::tagged_message::tagged_body;
use super::{ChatItem, EventKind, UserContentPart};

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

pub(super) fn chat_item(prompt: &WatchWebQueuedPrompt) -> ChatItem {
    let content = strip_id_marker(&prompt.content);
    if let Some((tag, body)) = tagged_body(&content)
        && (!action_event::is_vibe_mode_tag(tag)
            || prompt
                .message_id
                .as_deref()
                .is_some_and(action_event::is_vibe_mode_message_id))
        && let Some(text) = action_event::event_text(tag, body)
    {
        return ChatItem::Event {
            kind: EventKind::Armyknife,
            text,
            failed: false,
            timestamp: Some(prompt.sent_at.to_rfc3339()),
        };
    }

    ChatItem::QueuedUser {
        content: user_content(prompt),
        queued_prompt_id: prompt.message_id.clone(),
        from: None,
        timestamp: Some(prompt.sent_at.to_rfc3339()),
    }
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
            ChatItem::Event { text, .. } => remove_id_marker(text),
            _ => None,
        };
        let Some(message_id) = message_id else {
            continue;
        };
        let Some(prompt) = prompts_by_message_id.get(message_id.as_str()).copied() else {
            continue;
        };
        if matches!(message, ChatItem::Event { .. }) {
            if !prompt.resolved {
                resolved.push(prompt.id);
            }
            continue;
        }
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
            .map(chat_item),
    );
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use rstest::rstest;

    use super::{
        add_id_marker, append_unresolved_prompts, chat_item, id_from_content,
        resolve_transcript_messages, strip_id_marker,
    };
    use crate::commands::agent::store::{WatchWebQueuedAttachment, WatchWebQueuedPrompt};
    use crate::commands::agent::watch_web::messages::{ChatItem, EventKind, UserContentPart};

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

    #[rstest]
    #[case::vibe_on(
        "armyknife-vibe-mode-on",
        "watch-web からユーザーが vibe mode を on にしました。",
        "Vibe mode turned on"
    )]
    #[case::vibe_off(
        "armyknife-vibe-mode-off",
        "watch-web からユーザーが vibe mode を off にしました。祖先セッションから引き継いでいる場合、vibe mode は引き続き有効です。",
        "Vibe mode turned off"
    )]
    fn tagged_vibe_prompts_are_events_while_queued(
        #[case] tag: &str,
        #[case] body: &str,
        #[case] expected_text: &str,
    ) {
        let prompt = WatchWebQueuedPrompt {
            id: 7,
            message_id: Some("armyknife-vibe-mode:fixture-message-cyan".to_owned()),
            content: format!("<{tag}>{body}</{tag}>"),
            attachments: Vec::<WatchWebQueuedAttachment>::new(),
            resolved: false,
            sent_at: Utc::now(),
        };

        assert_eq!(
            {
                let mut messages = Vec::new();
                append_unresolved_prompts(&mut messages, std::slice::from_ref(&prompt), &[]);
                messages
            },
            vec![ChatItem::Event {
                kind: EventKind::Armyknife,
                text: expected_text.to_owned(),
                failed: false,
                timestamp: Some(prompt.sent_at.to_rfc3339()),
            }],
        );
    }

    #[test]
    fn user_text_with_a_vibe_tag_stays_queued_user_content() {
        let prompt = WatchWebQueuedPrompt {
            id: 7,
            message_id: Some("fixture-message-cyan".to_owned()),
            content: "<armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>"
                .to_owned(),
            attachments: Vec::<WatchWebQueuedAttachment>::new(),
            resolved: false,
            sent_at: Utc::now(),
        };

        assert_eq!(
            chat_item(&prompt),
            ChatItem::QueuedUser {
                content: vec![UserContentPart::Text {
                    text: prompt.content.clone(),
                }],
                queued_prompt_id: Some("fixture-message-cyan".to_owned()),
                from: None,
                timestamp: Some(prompt.sent_at.to_rfc3339()),
            },
        );
    }

    #[test]
    fn resolved_event_prompt_is_not_marked_resolved_again() {
        let prompt = WatchWebQueuedPrompt {
            id: 7,
            message_id: Some("armyknife-vibe-mode:fixture-message-cyan".to_owned()),
            content: "<armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>"
                .to_owned(),
            attachments: Vec::<WatchWebQueuedAttachment>::new(),
            resolved: true,
            sent_at: Utc::now(),
        };
        let mut messages = [ChatItem::Event {
            kind: EventKind::Armyknife,
            text: add_id_marker(
                "Vibe mode turned on",
                "armyknife-vibe-mode:fixture-message-cyan",
            ),
            failed: false,
            timestamp: Some(prompt.sent_at.to_rfc3339()),
        }];

        assert_eq!(
            resolve_transcript_messages(&mut messages, &[prompt]),
            Vec::<i64>::new(),
        );
    }
}
