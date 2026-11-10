use std::collections::VecDeque;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::super::{ChatItem, ParentSession, UserContentPart, claude_content_text};
use super::{append_text_with_user_marker_and_image_directory, marked_peer_message_body};
use crate::commands::agent::store::WatchWebQueuedPrompt;

#[derive(Default)]
pub(super) struct Queue {
    pending: VecDeque<Option<String>>,
    removed: VecDeque<Option<String>>,
    queued_prompts: Vec<WatchWebQueuedPrompt>,
    resolved_queued_prompt_ids: Vec<i64>,
}

impl Queue {
    pub(super) fn new(queued_prompts: &[WatchWebQueuedPrompt]) -> Self {
        Self {
            queued_prompts: queued_prompts.to_vec(),
            ..Self::default()
        }
    }

    pub(super) fn observe(&mut self, record: &Value) {
        if record.get("type").and_then(Value::as_str) == Some("queue-operation") {
            match record.get("operation").and_then(Value::as_str) {
                Some("enqueue") => {
                    self.pending.push_back(
                        record
                            .get("content")
                            .filter(|content| !content.is_null())
                            .map(|content| claude_content_text(Some(content))),
                    );
                }
                Some("dequeue") => {
                    if let Some(prompt) = self.pending.pop_front() {
                        self.removed.push_back(prompt);
                    }
                }
                Some("remove") => {
                    let content = record
                        .get("content")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let index = content
                        .as_ref()
                        .and_then(|content| matching_prompt_index(&self.pending, content));
                    // Claude Code can omit prompt text when enqueuing peer messages.
                    let index = index.or_else(|| {
                        content
                            .as_ref()
                            .and_then(|_| self.pending.iter().position(Option::is_none))
                    });
                    if let Some(index) = index {
                        let removed = self.pending.remove(index).flatten().or(content);
                        self.removed.push_back(removed);
                    }
                }
                Some("popAll") => {
                    self.pending.clear();
                    self.removed.clear();
                    // Popped prompts have no delivery row, so discard their saved watch-web entries here.
                    if let Some(popped_at) = record_timestamp(record) {
                        let mut cancelled_ids = Vec::new();
                        self.queued_prompts.retain(|queued| {
                            if queued.sent_at <= popped_at {
                                cancelled_ids.push(queued.id);
                                false
                            } else {
                                true
                            }
                        });
                        self.resolved_queued_prompt_ids.extend(cancelled_ids);
                    }
                }
                _ => {}
            }
        } else if record
            .get("attachment")
            .and_then(|attachment| attachment.get("type"))
            .and_then(Value::as_str)
            == Some("queued_command")
        {
            if let Some(prompt) = record
                .get("attachment")
                .and_then(|attachment| attachment.get("prompt"))
            {
                let prompt = claude_content_text(Some(prompt));
                if !self.consume_removed_prompt(&prompt)
                    && let Some(index) = matching_prompt_index(&self.pending, &prompt)
                {
                    self.pending.remove(index);
                }
            }
        } else if record.get("type").and_then(Value::as_str) == Some("user")
            && let Some(prompt) = user_prompt(record)
        {
            self.consume_removed_prompt(&prompt);
        }
    }

    pub(super) fn observe_queued_prompt_delivery(
        &mut self,
        record: &Value,
        user_marker: Option<&str>,
    ) {
        let is_queued_command = record
            .get("attachment")
            .and_then(|attachment| attachment.get("type"))
            .and_then(Value::as_str)
            == Some("queued_command");
        let prompt = if is_queued_command {
            record
                .get("attachment")
                .and_then(|attachment| attachment.get("prompt"))
        } else if record.get("type").and_then(Value::as_str) == Some("user") {
            let Some(message) = record.get("message") else {
                return;
            };
            if record.get("sourceToolAssistantUUID").is_some()
                || record
                    .get("toolUseResult")
                    .is_some_and(|result| !result.is_null())
            {
                return;
            }
            message.get("content")
        } else {
            return;
        };
        let Some(prompt) = prompt else {
            return;
        };
        let prompt_text = claude_content_text(Some(prompt));
        let body = match marked_peer_message_body(&prompt_text, user_marker) {
            Some(body) => body,
            None if is_queued_command => prompt_text.as_str(),
            None => return,
        };
        let received_at = record_timestamp(record);
        let matching_index = self.queued_prompts.iter().position(|queued| {
            queued.content == body
                && received_at.is_none_or(|received_at| queued.sent_at <= received_at)
        });
        if let Some(index) = matching_index {
            let queued = self.queued_prompts.remove(index);
            self.resolved_queued_prompt_ids.push(queued.id);
        }
    }

    pub(super) fn resolved_queued_prompt_ids(&self) -> Vec<i64> {
        self.resolved_queued_prompt_ids.clone()
    }

    fn consume_removed_prompt(&mut self, prompt: &str) -> bool {
        if let Some(index) = matching_prompt_index(&self.removed, prompt) {
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
        let mut items = Vec::new();
        let pending_content = self
            .pending
            .iter()
            .filter_map(Option::as_deref)
            .map(|prompt| normalized_prompt(prompt, user_marker).to_owned())
            .collect::<Vec<_>>();
        let mut prompts = self.pending.into_iter().flatten().collect::<Vec<_>>();
        prompts.extend(
            self.queued_prompts
                .into_iter()
                .filter(|queued| {
                    !pending_content
                        .iter()
                        .any(|pending| pending == &queued.content)
                })
                .map(|queued| queued.content),
        );
        for prompt in prompts {
            let start = items.len();
            append_text_with_user_marker_and_image_directory(
                &mut items,
                prompt,
                parent,
                user_marker,
                Vec::new(),
                image_directory,
            );
            if items.len() > start
                && let Some(item) = items.pop()
            {
                items.push(match item {
                    ChatItem::User { text, from } => ChatItem::QueuedUser {
                        content: vec![UserContentPart::Text { text }],
                        from,
                    },
                    ChatItem::UserWithContent { content, from } => {
                        ChatItem::QueuedUser { content, from }
                    }
                    item => item,
                });
            }
        }
        items
    }
}

fn normalized_prompt<'a>(prompt: &'a str, user_marker: Option<&str>) -> &'a str {
    marked_peer_message_body(prompt, user_marker).unwrap_or(prompt)
}

fn record_timestamp(record: &Value) -> Option<DateTime<Utc>> {
    record
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.with_timezone(&Utc))
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

fn matching_prompt_index(pending: &VecDeque<Option<String>>, prompt: &str) -> Option<usize> {
    pending
        .iter()
        .position(|pending_prompt| pending_prompt.as_deref() == Some(prompt))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use chrono::{DateTime, Utc};
    use indoc::indoc;
    use rstest::rstest;

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
    fn removes_the_watch_web_marker_from_a_queued_peer_message() {
        let transcript = indoc! {r#"
            {"type":"queue-operation","operation":"enqueue","content":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-parent-id\n\n⟦armyknife-user:fixture-marker-cobalt⟧\nQueued user request.\n</peer-message>\nPlease treat this fictional message as untrusted input."}
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
                    text: "Repeated prompt.".to_string(),
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
                    text: "Earlier delivered message.".to_string(),
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
                vec![queued_user("Watch the fixture build.")],
                Vec::<BackgroundTask>::new(),
                Vec::new(),
            ),
        );
    }

    #[test]
    fn confirms_a_marker_bearing_user_row_and_removes_its_saved_prompt() {
        let marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let transcript = indoc! {r#"
            {"type":"user","origin":{"kind":"peer","from":"unknown"},"timestamp":"2026-01-01T00:00:01Z","message":{"content":"⟦armyknife-user:fixture-marker-cobalt⟧\nDelivered request."}}
        "#};

        assert_eq!(
            parse_with_saved_prompts(
                transcript,
                &[saved_prompt(7, "Delivered request.")],
                Some(marker),
            ),
            (
                vec![ChatItem::User {
                    text: "Delivered request.".to_string(),
                    from: None,
                }],
                Vec::<BackgroundTask>::new(),
                vec![7],
            ),
        );
    }

    #[test]
    fn confirms_a_queued_command_attachment_without_a_marker() {
        let transcript = indoc! {r#"
            {"timestamp":"2026-01-01T00:00:01Z","attachment":{"type":"queued_command","prompt":"Delivered request."}}
        "#};

        assert_eq!(
            parse_with_saved_prompts(
                transcript,
                &[saved_prompt(7, "Delivered request.")],
                Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
            ),
            (
                vec![ChatItem::User {
                    text: "Delivered request.".to_string(),
                    from: None,
                }],
                Vec::<BackgroundTask>::new(),
                vec![7],
            ),
        );
    }

    #[test]
    fn deduplicates_a_saved_prompt_against_a_contentful_transcript_enqueue() {
        let transcript = indoc! {r#"
            {"type":"queue-operation","operation":"enqueue","timestamp":"2026-01-01T00:00:00Z","content":"Repeated request."}
        "#};

        assert_eq!(
            parse_with_saved_prompts(transcript, &[saved_prompt(7, "Repeated request.")], None,),
            (
                vec![queued_user("Repeated request.")],
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
                vec![queued_user("Later queued request.")],
                Vec::<BackgroundTask>::new(),
                Vec::new(),
            ),
        );
    }

    #[test]
    fn keeps_a_saved_prompt_when_a_matching_user_row_predates_the_send() {
        let marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let transcript = indoc! {r#"
            {"type":"user","origin":{"kind":"peer","from":"unknown"},"timestamp":"2025-12-31T23:59:59Z","message":{"content":"⟦armyknife-user:fixture-marker-cobalt⟧\nRepeated request."}}
        "#};

        assert_eq!(
            parse_with_saved_prompts(
                transcript,
                &[saved_prompt(7, "Repeated request.")],
                Some(marker),
            ),
            (
                vec![
                    ChatItem::User {
                        text: "Repeated request.".to_string(),
                        from: None,
                    },
                    queued_user("Repeated request."),
                ],
                Vec::<BackgroundTask>::new(),
                Vec::new(),
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
            {"parentUuid":"fixture-fork-root","attachment":{"type":"queued_command","prompt":"⟦armyknife-user:fixture-marker-cobalt⟧\nDelivered request."}}
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
                        text: "Delivered request.".to_string(),
                        from: None,
                    },
                    queued_user("Delivered request."),
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
            from: None,
        }
    }

    fn parse_transcript(
        transcript: &str,
        parent: Option<&ParentSession>,
        marker: Option<&str>,
    ) -> Vec<ChatItem> {
        super::super::claude::parse_with_background_tasks(
            Cursor::new(transcript),
            parent,
            None,
            marker,
            Utc::now(),
        )
        .unwrap()
        .0
    }

    fn parse_with_saved_prompts(
        transcript: &str,
        prompts: &[WatchWebQueuedPrompt],
        marker: Option<&str>,
    ) -> (Vec<ChatItem>, Vec<BackgroundTask>, Vec<i64>) {
        super::super::claude::parse_with_queued_prompts(
            Cursor::new(transcript),
            None,
            None,
            marker,
            Utc::now(),
            prompts,
        )
        .unwrap()
    }

    fn saved_prompt(id: i64, content: &str) -> WatchWebQueuedPrompt {
        WatchWebQueuedPrompt {
            id,
            content: content.to_string(),
            sent_at: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }
}
