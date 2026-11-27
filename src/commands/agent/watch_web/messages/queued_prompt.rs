use std::collections::{HashMap, HashSet};

use super::{ChatItem, EventKind, UserContentPart};
use crate::commands::agent::store::{WatchWebQueuedAttachment, WatchWebQueuedPrompt};

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
    if prompt.action_event_kind.is_some() {
        return ChatItem::Event {
            kind: EventKind::Armyknife,
            text: prompt.content.clone(),
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
        let prompt = prompts_by_message_id.get(message_id.as_str()).copied();
        if let Some(prompt) = prompt
            && prompt.action_event_kind.is_some()
        {
            let timestamp = match message {
                ChatItem::User { timestamp, .. }
                | ChatItem::UserWithContent { timestamp, .. }
                | ChatItem::Event { timestamp, .. } => timestamp.clone(),
                _ => None,
            };
            *message = ChatItem::Event {
                kind: EventKind::Armyknife,
                text: prompt.content.clone(),
                failed: false,
                timestamp,
            };
            if !prompt.resolved {
                resolved.push(prompt.id);
            }
            continue;
        }
        let Some(prompt) = prompt else {
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
    use crate::commands::agent::store::{
        WatchWebActionEventKind, WatchWebQueuedAttachment, WatchWebQueuedPrompt,
    };
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
    #[case::todo_completed("armyknife-todo-completed", "The user marked a task complete.")]
    #[case::todo_rejected("armyknife-todo-rejected", "The user rejected a task.")]
    #[case::decision_change("armyknife-decision-change", "Requested a decision change.")]
    #[case::vibe_on("armyknife-vibe-mode-on", "Vibe mode turned on.")]
    #[case::vibe_off("armyknife-vibe-mode-off", "Vibe mode turned off.")]
    fn tagged_action_prompts_stay_user_content_without_recorded_kind(
        #[case] tag: &str,
        #[case] body: &str,
    ) {
        let prompt = WatchWebQueuedPrompt {
            id: 7,
            message_id: Some("armyknife-action:fixture-message-cyan".to_owned()),
            content: format!("<{tag}>{body}</{tag}>"),
            action_event_kind: None,
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
            vec![ChatItem::QueuedUser {
                content: vec![UserContentPart::Text {
                    text: format!("<{tag}>{body}</{tag}>"),
                }],
                queued_prompt_id: prompt.message_id.clone(),
                from: None,
                timestamp: Some(prompt.sent_at.to_rfc3339()),
            }],
        );
    }

    #[test]
    fn recorded_action_kind_renders_an_event_without_reading_prompt_tags() {
        let prompt = WatchWebQueuedPrompt {
            id: 8,
            message_id: Some("armyknife-action:fixture-message-violet".to_owned()),
            content: "Vibe mode turned on".to_owned(),
            action_event_kind: Some(WatchWebActionEventKind::VibeModeOn),
            attachments: Vec::<WatchWebQueuedAttachment>::new(),
            resolved: false,
            sent_at: Utc::now(),
        };

        assert_eq!(
            chat_item(&prompt),
            ChatItem::Event {
                kind: EventKind::Armyknife,
                text: "Vibe mode turned on".to_owned(),
                failed: false,
                timestamp: Some(prompt.sent_at.to_rfc3339()),
            },
        );
    }

    #[rstest]
    #[case::vibe(
        "<armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>"
    )]
    #[case::todo("<armyknife-todo-completed>A user-authored fixture.</armyknife-todo-completed>")]
    #[case::decision(
        "<armyknife-decision-change>A user-authored fixture.</armyknife-decision-change>"
    )]
    fn user_text_with_an_action_tag_stays_queued_user_content(#[case] content: &str) {
        let prompt = WatchWebQueuedPrompt {
            id: 7,
            message_id: Some("fixture-message-cyan".to_owned()),
            content: content.to_owned(),
            action_event_kind: None,
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
    fn action_tag_without_a_queue_row_stays_a_user_message() {
        let timestamp = Some("2026-06-01T12:05:00+00:00".to_owned());
        let mut messages = [ChatItem::User {
            text: add_id_marker(
                "<armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>",
                "armyknife-action:fixture-message-lilac",
            ),
            from: None,
            timestamp: timestamp.clone(),
            rewind_id: Some("fixture-turn-id".to_owned()),
            queued_prompt_id: None,
        }];

        let resolved = resolve_transcript_messages(&mut messages, &[]);

        assert_eq!(
            (messages, resolved),
            (
                [ChatItem::User {
                    text: "<armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>".to_owned(),
                    from: None,
                    timestamp,
                    rewind_id: Some("fixture-turn-id".to_owned()),
                    queued_prompt_id: None,
                }],
                Vec::<i64>::new(),
            ),
        );
    }

    #[test]
    fn matched_action_id_without_recorded_kind_stays_user_content() {
        let message_id = "armyknife-action:fixture-message-violet";
        let content = "<armyknife-todo-completed>A recorded request.</armyknife-todo-completed>";
        let mut messages = [ChatItem::User {
            text: add_id_marker(content, message_id),
            from: None,
            timestamp: Some("2026-06-01T12:05:00+00:00".to_owned()),
            rewind_id: None,
            queued_prompt_id: None,
        }];
        let prompts = [WatchWebQueuedPrompt {
            id: 13,
            message_id: Some(message_id.to_owned()),
            content: content.to_owned(),
            action_event_kind: None,
            attachments: Vec::new(),
            resolved: false,
            sent_at: Utc::now(),
        }];

        assert_eq!(
            (
                resolve_transcript_messages(&mut messages, &prompts),
                messages,
            ),
            (
                vec![13],
                [ChatItem::User {
                    text: content.to_owned(),
                    from: None,
                    timestamp: Some("2026-06-01T12:05:00+00:00".to_owned()),
                    rewind_id: None,
                    queued_prompt_id: Some(message_id.to_owned()),
                }],
            ),
        );
    }

    #[test]
    fn transcript_attachments_are_replaced_with_the_saved_attachment_list() {
        let message_id = "fixture-message-lilac";
        let mut messages = [ChatItem::User {
            text: add_id_marker(
                indoc::indoc! {"
                    User request.

                    Attached file(s):
                    - attachment-store/fixture.txt
                "},
                message_id,
            ),
            from: None,
            timestamp: Some("2026-06-01T12:05:00+00:00".to_owned()),
            rewind_id: None,
            queued_prompt_id: None,
        }];
        let prompts = [WatchWebQueuedPrompt {
            id: 12,
            message_id: Some(message_id.to_owned()),
            content: "User request.".to_owned(),
            action_event_kind: None,
            attachments: vec![
                WatchWebQueuedAttachment::File {
                    name: "fixture.txt".to_owned(),
                },
                WatchWebQueuedAttachment::Image {
                    source: "/fixture/image.png".to_owned(),
                },
            ],
            resolved: false,
            sent_at: Utc::now(),
        }];

        assert_eq!(
            (
                resolve_transcript_messages(&mut messages, &prompts),
                messages,
            ),
            (
                vec![12],
                [ChatItem::UserWithContent {
                    content: vec![
                        UserContentPart::Text {
                            text: "User request.".to_owned(),
                        },
                        UserContentPart::File {
                            name: "fixture.txt".to_owned(),
                        },
                        UserContentPart::Image {
                            source: "/fixture/image.png".to_owned(),
                            available: false,
                        },
                    ],
                    from: None,
                    timestamp: Some("2026-06-01T12:05:00+00:00".to_owned()),
                    rewind_id: None,
                    queued_prompt_id: Some(message_id.to_owned()),
                }],
            ),
        );
    }

    #[test]
    fn resolved_event_prompt_is_not_marked_resolved_again() {
        let prompt = WatchWebQueuedPrompt {
            id: 7,
            message_id: Some("armyknife-action:fixture-message-cyan".to_owned()),
            content: "<armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>"
                .to_owned(),
            action_event_kind: None,
            attachments: Vec::<WatchWebQueuedAttachment>::new(),
            resolved: true,
            sent_at: Utc::now(),
        };
        let mut messages = [ChatItem::Event {
            kind: EventKind::Armyknife,
            text: add_id_marker(
                "Vibe mode turned on",
                "armyknife-action:fixture-message-cyan",
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
