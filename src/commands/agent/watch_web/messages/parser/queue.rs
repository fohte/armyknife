use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::super::{ChatItem, ParentSession, UserContentPart, claude_content_text};
use super::append_text_with_user_marker_and_image_directory;
use super::classifier::marked_user_body;
use crate::commands::agent::store::WatchWebQueuedPrompt;

#[derive(Clone)]
pub(super) struct QueueReplayEvent {
    kind: QueueRecordKind,
    timestamp: Option<DateTime<Utc>>,
    prompt: Option<String>,
    source_tool_assistant_uuid_present: bool,
    has_tool_use_result: bool,
    pub(super) active_branch_delivery: bool,
}

#[derive(Clone, Copy)]
enum QueueRecordKind {
    Operation(QueueOperation),
    QueuedCommand,
    User,
}

#[derive(Clone, Copy)]
enum QueueOperation {
    Enqueue,
    Dequeue,
    Remove,
    PopAll,
}

impl QueueReplayEvent {
    pub(super) fn from_record(record: &Value) -> Option<Self> {
        let kind = record.get("type").and_then(Value::as_str);
        let attachment_type = record
            .get("attachment")
            .and_then(|attachment| attachment.get("type"))
            .and_then(Value::as_str);
        let timestamp = super::record_timestamp(record);

        if kind == Some("queue-operation") {
            let operation = match record.get("operation").and_then(Value::as_str) {
                Some("enqueue") => QueueOperation::Enqueue,
                Some("dequeue") => QueueOperation::Dequeue,
                Some("remove") => QueueOperation::Remove,
                Some("popAll") => QueueOperation::PopAll,
                _ => return None,
            };
            let prompt = match operation {
                QueueOperation::Enqueue => record
                    .get("content")
                    .filter(|content| !content.is_null())
                    .map(|content| claude_content_text(Some(content))),
                QueueOperation::Remove => record
                    .get("content")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                QueueOperation::Dequeue | QueueOperation::PopAll => None,
            };
            return Some(Self {
                kind: QueueRecordKind::Operation(operation),
                timestamp,
                prompt,
                source_tool_assistant_uuid_present: false,
                has_tool_use_result: false,
                active_branch_delivery: false,
            });
        }

        if attachment_type == Some("queued_command") {
            return Some(Self {
                kind: QueueRecordKind::QueuedCommand,
                timestamp,
                prompt: record
                    .get("attachment")
                    .and_then(|attachment| attachment.get("prompt"))
                    .map(|prompt| claude_content_text(Some(prompt))),
                source_tool_assistant_uuid_present: false,
                has_tool_use_result: false,
                active_branch_delivery: false,
            });
        }

        if kind == Some("user") {
            return Some(Self {
                kind: QueueRecordKind::User,
                timestamp,
                prompt: user_prompt(record),
                source_tool_assistant_uuid_present: record.get("sourceToolAssistantUUID").is_some(),
                has_tool_use_result: record
                    .get("toolUseResult")
                    .is_some_and(|result| !result.is_null()),
                active_branch_delivery: false,
            });
        }

        None
    }
}

#[derive(Clone, Default)]
pub(super) struct Queue {
    pending: VecDeque<PendingPrompt>,
    removed: VecDeque<Option<String>>,
    queued_prompts: Vec<WatchWebQueuedPrompt>,
    queued_prompt_ids_by_message_id: HashMap<String, usize>,
    delivered_prompt_ids: HashSet<i64>,
    resolved_queued_prompt_ids: Vec<i64>,
}

#[derive(Clone)]
struct PendingPrompt {
    content: Option<String>,
    timestamp: Option<DateTime<Utc>>,
}

impl Queue {
    pub(super) fn new(queued_prompts: &[WatchWebQueuedPrompt]) -> Self {
        Self {
            queued_prompts: queued_prompts.to_vec(),
            queued_prompt_ids_by_message_id: queued_prompts
                .iter()
                .enumerate()
                .filter_map(|(index, prompt)| {
                    prompt
                        .message_id
                        .as_ref()
                        .map(|message_id| (message_id.clone(), index))
                })
                .collect(),
            ..Self::default()
        }
    }

    pub(super) fn observe(&mut self, event: &QueueReplayEvent) {
        match event.kind {
            QueueRecordKind::Operation(operation) => match operation {
                QueueOperation::Enqueue => {
                    self.pending.push_back(PendingPrompt {
                        content: event.prompt.clone(),
                        timestamp: event.timestamp,
                    });
                }
                QueueOperation::Dequeue => {
                    if let Some(prompt) = self.pending.pop_front() {
                        self.removed.push_back(prompt.content);
                    }
                }
                QueueOperation::Remove => {
                    let content = event.prompt.clone();
                    let index = content
                        .as_ref()
                        .and_then(|content| pending_prompt_index(&self.pending, content));
                    // Claude Code can omit prompt text when enqueuing peer messages.
                    let index = index.or_else(|| {
                        content.as_ref().and_then(|_| {
                            self.pending
                                .iter()
                                .position(|prompt| prompt.content.is_none())
                        })
                    });
                    if let Some(index) = index {
                        let removed = self
                            .pending
                            .remove(index)
                            .and_then(|prompt| prompt.content)
                            .or(content);
                        self.removed.push_back(removed);
                    }
                }
                QueueOperation::PopAll => {
                    self.pending.clear();
                    self.removed.clear();
                    // Popped prompts have no delivery record, so stop showing them as pending.
                    if let Some(popped_at) = event.timestamp {
                        for queued in &mut self.queued_prompts {
                            if !queued.resolved && queued.sent_at <= popped_at {
                                queued.resolved = true;
                                self.resolved_queued_prompt_ids.push(queued.id);
                            }
                        }
                    }
                }
            },
            QueueRecordKind::QueuedCommand => {
                if let Some(prompt) = event.prompt.as_deref()
                    && !self.consume_removed_prompt(prompt)
                    && let Some(index) = pending_prompt_index(&self.pending, prompt)
                {
                    self.pending.remove(index);
                }
            }
            QueueRecordKind::User => {
                if let Some(prompt) = event.prompt.as_deref() {
                    self.consume_removed_prompt(prompt);
                }
            }
        }
    }

    pub(super) fn observe_queued_prompt_delivery(
        &mut self,
        event: &QueueReplayEvent,
        user_marker: Option<&str>,
    ) {
        let is_queued_command = matches!(event.kind, QueueRecordKind::QueuedCommand);
        if matches!(event.kind, QueueRecordKind::Operation(_))
            || (matches!(event.kind, QueueRecordKind::User)
                && (event.source_tool_assistant_uuid_present || event.has_tool_use_result))
        {
            return;
        }
        let Some(prompt_text) = event.prompt.as_deref() else {
            return;
        };
        let body = match marked_user_body(prompt_text, user_marker) {
            Some(body) => body,
            None if is_queued_command => prompt_text.to_owned(),
            None => return,
        };
        let Some(message_id) = super::super::queued_prompt::id_from_content(&body) else {
            return;
        };
        if let Some(index) = self
            .queued_prompt_ids_by_message_id
            .get(message_id)
            .copied()
        {
            let queued = &self.queued_prompts[index];
            self.delivered_prompt_ids.insert(queued.id);
            if !queued.resolved {
                self.resolved_queued_prompt_ids.push(queued.id);
            }
        }
    }

    pub(super) fn resolved_queued_prompt_ids(&self) -> Vec<i64> {
        self.resolved_queued_prompt_ids.clone()
    }

    fn consume_removed_prompt(&mut self, prompt: &str) -> bool {
        if let Some(index) = matching_removed_prompt_index(&self.removed, prompt) {
            self.removed.remove(index);
            return true;
        }

        if let Some(index) = self.removed.iter().position(Option::is_none) {
            self.removed.remove(index);
            return true;
        }

        false
    }

    pub(super) fn pending_items(
        self,
        parent: Option<&ParentSession>,
        user_marker: Option<&str>,
        image_directory: Option<&Path>,
    ) -> Vec<ChatItem> {
        let Queue {
            pending,
            queued_prompts,
            queued_prompt_ids_by_message_id,
            delivered_prompt_ids,
            ..
        } = self;
        let mut items = Vec::new();
        let prompts = pending
            .into_iter()
            .filter_map(|prompt| prompt.content.map(|content| (content, prompt.timestamp)))
            .collect::<Vec<_>>();
        for (prompt, timestamp) in prompts {
            let message_id = super::super::queued_prompt::id_from_content(&prompt);
            let is_saved_prompt =
                message_id.is_some_and(|id| queued_prompt_ids_by_message_id.contains_key(id));
            let prompt = normalized_prompt(&prompt, user_marker);
            if is_saved_prompt {
                continue;
            }
            let start = items.len();
            append_text_with_user_marker_and_image_directory(
                &mut items,
                prompt,
                parent,
                user_marker,
                Vec::new(),
                image_directory,
                timestamp,
            );
            if items.len() > start
                && let Some(item) = items.pop()
            {
                items.push(match item {
                    ChatItem::User {
                        text,
                        from,
                        timestamp,
                        ..
                    } => ChatItem::QueuedUser {
                        content: vec![UserContentPart::Text { text }],
                        from,
                        timestamp,
                    },
                    ChatItem::UserWithContent {
                        content,
                        from,
                        timestamp,
                        ..
                    } => ChatItem::QueuedUser {
                        content,
                        from,
                        timestamp,
                    },
                    item => item,
                });
            }
        }
        for queued in queued_prompts
            .into_iter()
            .filter(|queued| !queued.resolved && !delivered_prompt_ids.contains(&queued.id))
        {
            items.push(ChatItem::QueuedUser {
                content: super::super::queued_prompt::user_content(&queued),
                from: None,
                timestamp: Some(queued.sent_at.to_rfc3339()),
            });
        }
        items
    }
}

fn normalized_prompt(prompt: &str, user_marker: Option<&str>) -> String {
    let prompt = marked_user_body(prompt, user_marker).unwrap_or_else(|| prompt.to_owned());
    super::super::queued_prompt::strip_id_marker(&prompt)
}

fn user_prompt(record: &Value) -> Option<String> {
    let content = record.get("message")?.get("content")?;
    if let Some(blocks) = content.as_array()
        && blocks
            .iter()
            .all(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
    {
        return None;
    }
    let text = claude_content_text(Some(content));
    (!text.trim().is_empty()).then_some(text)
}

fn matching_removed_prompt_index(
    pending: &VecDeque<Option<String>>,
    prompt: &str,
) -> Option<usize> {
    pending
        .iter()
        .position(|pending_prompt| pending_prompt.as_deref() == Some(prompt))
}

fn pending_prompt_index(pending: &VecDeque<PendingPrompt>, prompt: &str) -> Option<usize> {
    pending
        .iter()
        .position(|pending_prompt| pending_prompt.content.as_deref() == Some(prompt))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use chrono::{DateTime, Utc};
    use indoc::{formatdoc, indoc};
    use rstest::rstest;

    use super::super::claude::ParserAdditions;
    use crate::commands::agent::store::WatchWebQueuedPrompt;
    use crate::commands::agent::watch_web::messages::{
        BackgroundTask, ChatItem, ParentSession, UserContentPart,
    };

    #[rstest]
    #[case::dequeue_consumes_the_oldest_item(
        indoc! {r#"
            {"type":"queue-operation","operation":"enqueue","content":"First queued prompt."}
            {"type":"queue-operation","operation":"enqueue","content":null}
            {"type":"queue-operation","operation":"enqueue","content":"Next queued prompt."}
            {"type":"queue-operation","operation":"dequeue"}
        "#},
        vec![queued_user("Next queued prompt.")]
    )]
    #[case::remove_consumes_matching_item(
        indoc! {r#"
            {"type":"queue-operation","operation":"enqueue","content":"Remove this prompt."}
            {"type":"queue-operation","operation":"enqueue","content":"Keep this prompt."}
            {"type":"queue-operation","operation":"remove","reason":"absorbed_mid_turn","content":"Remove this prompt."}
        "#},
        vec![queued_user("Keep this prompt.")]
    )]
    #[case::remove_consumes_contentless_item(
        indoc! {r#"
            {"type":"queue-operation","operation":"enqueue","content":null}
            {"type":"queue-operation","operation":"remove","content":"Delivered prompt."}
        "#},
        vec![]
    )]
    #[case::pop_all_clears_the_pending_queue(
        indoc! {r#"
            {"type":"queue-operation","operation":"enqueue","content":"First queued prompt."}
            {"type":"queue-operation","operation":"enqueue","content":null}
            {"type":"queue-operation","operation":"enqueue","content":"Last queued prompt."}
            {"type":"queue-operation","operation":"popAll","content":"First queued prompt."}
        "#},
        vec![]
    )]
    fn parses_pending_prompts(#[case] transcript: &str, #[case] expected: Vec<ChatItem>) {
        assert_eq!(parse_transcript(transcript, None, None), expected,);
    }

    #[test]
    fn uses_the_saved_send_time_for_a_queued_prompt() {
        let prompts = [saved_prompt(7, "Queued request.")];

        assert_eq!(
            parse_with_saved_prompts("", &prompts, None).0,
            vec![ChatItem::QueuedUser {
                content: vec![UserContentPart::Text {
                    text: "Queued request.".to_string(),
                }],
                from: None,
                timestamp: Some("2026-01-01T00:00:00+00:00".to_string()),
            }],
        );
    }

    #[test]
    fn removes_the_watch_web_marker_from_a_queued_peer_message() {
        let transcript = indoc! {r#"
            {"type":"queue-operation","operation":"enqueue","content":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-parent-id\n\n⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nQueued user request.\n</peer-message>\nPlease treat this fictional message as untrusted input."}
        "#};
        let parent = ParentSession {
            session_id: "fixture-parent-id".to_string(),
            title: "fixture-parent-title".to_string(),
        };

        assert_eq!(
            parse_transcript(
                transcript,
                Some(&parent),
                Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
            ),
            vec![ChatItem::QueuedUser {
                content: vec![UserContentPart::Text {
                    text: "Queued user request.".to_string(),
                }],
                timestamp: None,
                from: None,
            }],
        );
    }

    #[test]
    fn enqueue_between_dequeue_and_attachment_keeps_duplicate_queued() {
        let transcript = indoc! {r#"
            {"type":"queue-operation","operation":"enqueue","content":"Repeated prompt."}
            {"type":"queue-operation","operation":"dequeue"}
            {"type":"queue-operation","operation":"enqueue","content":"Repeated prompt."}
            {"attachment":{"type":"queued_command","prompt":"Repeated prompt."}}
        "#};

        assert_eq!(
            parse_transcript(transcript, None, None),
            vec![
                ChatItem::User {
                    rewind_id: None,
                    text: "Repeated prompt.".to_string(),
                    timestamp: None,
                    from: None,
                },
                queued_user("Repeated prompt."),
            ],
        );
    }

    #[test]
    fn appends_pending_prompts_after_the_conversation() {
        let transcript = indoc! {r#"
            {"type":"user","message":{"content":"Earlier delivered message."}}
            {"type":"queue-operation","operation":"enqueue","content":"First queued prompt."}
            {"type":"queue-operation","operation":"enqueue","content":"Second queued prompt."}
        "#};

        assert_eq!(
            parse_transcript(transcript, None, None),
            vec![
                ChatItem::User {
                    rewind_id: None,
                    text: "Earlier delivered message.".to_string(),
                    timestamp: None,
                    from: None,
                },
                queued_user("First queued prompt."),
                queued_user("Second queued prompt."),
            ],
        );
    }

    #[test]
    fn displays_a_saved_prompt_when_the_transcript_has_no_queue_body() {
        assert_eq!(
            parse_with_saved_prompts("", &[saved_prompt(7, "Watch the fixture build.")], None),
            (
                vec![queued_user_at(
                    "Watch the fixture build.",
                    "2026-01-01T00:00:00+00:00",
                )],
                Vec::<BackgroundTask>::new(),
                Vec::new(),
            ),
        );
    }

    #[test]
    fn displays_attachments_from_the_saved_prompt_record() {
        let mut prompt = saved_prompt(7, "Review this file.");
        prompt.attachments = vec![
            crate::commands::agent::store::WatchWebQueuedAttachment::Image {
                source: "/api/images/fixture-image.png".to_owned(),
            },
            crate::commands::agent::store::WatchWebQueuedAttachment::File {
                name: "fixture-notes.txt".to_owned(),
            },
        ];

        assert_eq!(
            parse_with_saved_prompts("", &[prompt], None).0,
            vec![ChatItem::QueuedUser {
                content: vec![
                    UserContentPart::Text {
                        text: "Review this file.".to_owned(),
                    },
                    UserContentPart::Image {
                        source: "/api/images/fixture-image.png".to_owned(),
                        available: false,
                    },
                    UserContentPart::File {
                        name: "fixture-notes.txt".to_owned(),
                    },
                ],
                from: None,
                timestamp: Some("2026-01-01T00:00:00+00:00".to_owned()),
            }],
        );
    }

    #[test]
    fn resolves_a_delivered_prompt_and_preserves_its_rewind_id() {
        let marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let transcript = indoc! {r#"
            {"uuid":"fixture-user-message","parentUuid":null,"type":"user","origin":{"kind":"peer","from":"unknown"},"timestamp":"2026-01-01T00:00:01Z","message":{"content":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nEngine-formatted delivery.\n<!-- armyknife-queued-prompt-id:fixture-message-7 -->"}}
        "#};

        assert_eq!(
            parse_with_saved_prompts(
                transcript,
                &[saved_prompt(7, "Delivered request.")],
                Some(marker),
            ),
            (
                vec![ChatItem::User {
                    rewind_id: Some("fixture-user-message".to_string()),
                    text: "Delivered request.".to_string(),
                    timestamp: Some("2026-01-01T00:00:01+00:00".to_string()),
                    from: None,
                }],
                Vec::<BackgroundTask>::new(),
                vec![7],
            ),
        );
    }

    #[test]
    fn resolves_a_delivered_prompt_with_attachments_and_preserves_its_rewind_id() {
        let marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let transcript = indoc! {r#"
            {"uuid":"fixture-user-message","parentUuid":null,"type":"user","origin":{"kind":"peer","from":"unknown"},"timestamp":"2026-01-01T00:00:01Z","message":{"content":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nEngine-formatted delivery.\n<!-- armyknife-queued-prompt-id:fixture-message-7 -->"}}
        "#};
        let mut prompt = saved_prompt(7, "Delivered request.");
        prompt.attachments = vec![
            crate::commands::agent::store::WatchWebQueuedAttachment::Image {
                source: "/api/images/fixture-image.png".to_owned(),
            },
            crate::commands::agent::store::WatchWebQueuedAttachment::File {
                name: "fixture-notes.txt".to_owned(),
            },
        ];

        assert_eq!(
            parse_with_saved_prompts(transcript, &[prompt], Some(marker)),
            (
                vec![ChatItem::UserWithContent {
                    rewind_id: Some("fixture-user-message".to_owned()),
                    content: vec![
                        UserContentPart::Text {
                            text: "Delivered request.".to_owned(),
                        },
                        UserContentPart::Image {
                            source: "/api/images/fixture-image.png".to_owned(),
                            available: false,
                        },
                        UserContentPart::File {
                            name: "fixture-notes.txt".to_owned(),
                        },
                    ],
                    timestamp: Some("2026-01-01T00:00:01+00:00".to_owned()),
                    from: None,
                }],
                Vec::<BackgroundTask>::new(),
                vec![7],
            ),
        );
    }

    #[rstest]
    #[case::source_tool_assistant_uuid(
        serde_json::json!({"sourceToolAssistantUUID":"fixture-tool-row"})
    )]
    #[case::non_null_tool_use_result(
        serde_json::json!({"toolUseResult":{"result":"fixture output"}})
    )]
    fn user_tool_metadata_keeps_the_saved_prompt_pending(#[case] metadata: serde_json::Value) {
        let marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let mut record = serde_json::json!({
            "type": "user",
            "timestamp": "2026-01-01T00:00:01Z",
            "message": {
                "content": formatdoc! {"
                    {marker}
                    This message was typed by the human user in armyknife watch-web.
                    Delivered request.
                    <!-- armyknife-queued-prompt-id:fixture-message-7 -->
                "},
            },
        });
        record
            .as_object_mut()
            .expect("user record should be an object")
            .extend(
                metadata
                    .as_object()
                    .expect("test metadata should be an object")
                    .clone(),
            );
        let transcript = format!("{}\n", serde_json::to_string(&record).unwrap());

        assert_eq!(
            parse_with_saved_prompts(
                &transcript,
                &[saved_prompt(7, "Delivered request.")],
                Some(marker),
            ),
            (
                vec![queued_user_at(
                    "Delivered request.",
                    "2026-01-01T00:00:00+00:00",
                )],
                Vec::<BackgroundTask>::new(),
                Vec::new(),
            ),
        );
    }

    #[test]
    fn confirms_a_queued_command_attachment_by_its_id() {
        let transcript = indoc! {r#"
            {"timestamp":"2026-01-01T00:00:01Z","attachment":{"type":"queued_command","prompt":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nDelivered request.\n\n<!-- armyknife-queued-prompt-id:fixture-message-7 -->"}}
        "#};

        assert_eq!(
            parse_with_saved_prompts(
                transcript,
                &[saved_prompt(7, "Delivered request.")],
                Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
            ),
            (
                vec![ChatItem::User {
                    rewind_id: None,
                    text: "Delivered request.".to_string(),
                    timestamp: Some("2026-01-01T00:00:01+00:00".to_string()),
                    from: None,
                }],
                Vec::<BackgroundTask>::new(),
                vec![7],
            ),
        );
    }

    #[test]
    fn uses_the_saved_prompt_for_a_contentful_enqueue_with_the_same_id() {
        let transcript = indoc! {r#"
            {"type":"queue-operation","operation":"enqueue","timestamp":"2026-01-01T00:00:00Z","content":"Engine-formatted request.\n<!-- armyknife-queued-prompt-id:fixture-message-7 -->"}
        "#};

        assert_eq!(
            parse_with_saved_prompts(transcript, &[saved_prompt(7, "Repeated request.")], None,),
            (
                vec![queued_user_at(
                    "Repeated request.",
                    "2026-01-01T00:00:00+00:00",
                )],
                Vec::<BackgroundTask>::new(),
                Vec::new(),
            ),
        );
    }

    #[test]
    fn pop_all_cancels_saved_prompts_sent_before_it() {
        let transcript = indoc! {r#"
            {"type":"queue-operation","operation":"popAll","timestamp":"2026-01-01T00:00:01Z","content":"Cleared queued request."}
        "#};

        assert_eq!(
            parse_with_saved_prompts(
                transcript,
                &[saved_prompt(7, "Cleared queued request.")],
                None,
            ),
            (Vec::new(), Vec::<BackgroundTask>::new(), vec![7]),
        );
    }

    #[test]
    fn pop_all_keeps_saved_prompts_sent_after_it() {
        let transcript = indoc! {r#"
            {"type":"queue-operation","operation":"popAll","timestamp":"2025-12-31T23:59:59Z","content":"Earlier queued request."}
        "#};

        assert_eq!(
            parse_with_saved_prompts(
                transcript,
                &[saved_prompt(7, "Later queued request.")],
                None,
            ),
            (
                vec![queued_user_at(
                    "Later queued request.",
                    "2026-01-01T00:00:00+00:00",
                )],
                Vec::<BackgroundTask>::new(),
                Vec::new(),
            ),
        );
    }

    #[test]
    fn resolves_a_prompt_by_id_without_comparing_send_time() {
        let marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let transcript = indoc! {r#"
            {"type":"user","origin":{"kind":"peer","from":"unknown"},"timestamp":"2025-12-31T23:59:59Z","message":{"content":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nEngine-formatted request.\n<!-- armyknife-queued-prompt-id:fixture-message-7 -->"}}
        "#};

        assert_eq!(
            parse_with_saved_prompts(
                transcript,
                &[saved_prompt(7, "Repeated request.")],
                Some(marker),
            ),
            (
                vec![ChatItem::User {
                    rewind_id: None,
                    text: "Repeated request.".to_string(),
                    timestamp: Some("2025-12-31T23:59:59+00:00".to_string()),
                    from: None,
                }],
                Vec::<BackgroundTask>::new(),
                vec![7],
            ),
        );
    }

    #[test]
    fn keeps_a_saved_prompt_when_attachment_is_on_an_inactive_fork() {
        let marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let transcript = indoc! {r#"
            {"uuid":"fixture-active-root","parentUuid":null,"type":"assistant","message":{"content":[]}}
            {"uuid":"fixture-fork-root","parentUuid":"fixture-active-root","type":"assistant","message":{"content":[]}}
            {"uuid":"fixture-active-tip","parentUuid":"fixture-active-root","type":"assistant","message":{"content":[]}}
            {"parentUuid":"fixture-fork-root","attachment":{"type":"queued_command","prompt":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nDelivered request."}}
        "#};

        assert_eq!(
            parse_with_saved_prompts(
                transcript,
                &[saved_prompt(7, "Delivered request.")],
                Some(marker),
            ),
            (
                vec![
                    ChatItem::User {
                        rewind_id: None,
                        text: "Delivered request.".to_string(),
                        timestamp: None,
                        from: None,
                    },
                    queued_user_at("Delivered request.", "2026-01-01T00:00:00+00:00",),
                ],
                Vec::<BackgroundTask>::new(),
                Vec::new(),
            ),
        );
    }

    fn queued_user(text: &str) -> ChatItem {
        ChatItem::QueuedUser {
            content: vec![UserContentPart::Text {
                text: text.to_owned(),
            }],
            timestamp: None,
            from: None,
        }
    }

    fn queued_user_at(text: &str, timestamp: &str) -> ChatItem {
        ChatItem::QueuedUser {
            content: vec![UserContentPart::Text {
                text: text.to_owned(),
            }],
            timestamp: Some(timestamp.to_owned()),
            from: None,
        }
    }

    fn parse_transcript(
        transcript: &str,
        parent: Option<&ParentSession>,
        marker: Option<&str>,
    ) -> Vec<ChatItem> {
        super::super::claude::parse_with_queued_prompts_and_peer_out_messages_and_proposals(
            Cursor::new(transcript),
            Utc::now(),
            ParserAdditions {
                session_id: "fixture-session",
                parent,
                pending_tool: None,
                user_marker: marker,
                decisions: &[],
                queued_prompts: &[],
                peer_out_messages: &[],
                proposals: &[],
                todos: &[],
            },
        )
        .unwrap()
        .0
    }

    fn parse_with_saved_prompts(
        transcript: &str,
        prompts: &[WatchWebQueuedPrompt],
        marker: Option<&str>,
    ) -> (Vec<ChatItem>, Vec<BackgroundTask>, Vec<i64>) {
        let (mut messages, background_tasks, mut resolved_ids) =
            super::super::claude::parse_with_queued_prompts_and_peer_out_messages_and_proposals(
                Cursor::new(transcript),
                Utc::now(),
                ParserAdditions {
                    session_id: "fixture-session",
                    parent: None,
                    pending_tool: None,
                    user_marker: marker,
                    decisions: &[],
                    queued_prompts: prompts,
                    peer_out_messages: &[],
                    proposals: &[],
                    todos: &[],
                },
            )
            .unwrap();
        resolved_ids.extend(
            super::super::super::queued_prompt::resolve_transcript_messages(&mut messages, prompts),
        );
        resolved_ids.sort_unstable();
        resolved_ids.dedup();
        (messages, background_tasks, resolved_ids)
    }

    fn saved_prompt(id: i64, content: &str) -> WatchWebQueuedPrompt {
        WatchWebQueuedPrompt {
            id,
            message_id: Some(format!("fixture-message-{id}")),
            content: content.to_string(),
            attachments: vec![],
            resolved: false,
            sent_at: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }
}
