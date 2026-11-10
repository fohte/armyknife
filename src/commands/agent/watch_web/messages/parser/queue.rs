use std::collections::VecDeque;
use std::path::Path;

use serde_json::Value;

use super::super::{ChatItem, ParentSession, UserContentPart, claude_content_text};
use super::append_text_with_user_marker_and_image_directory;

#[derive(Default)]
pub(super) struct Queue {
    pending: VecDeque<Option<String>>,
    removed: VecDeque<Option<String>>,
}

impl Queue {
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
        for prompt in self.pending.into_iter().flatten() {
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

    use indoc::indoc;
    use rstest::rstest;

    use crate::commands::agent::watch_web::messages::{ChatItem, ParentSession, UserContentPart};

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
        assert_eq!(
            super::super::claude::parse(Cursor::new(transcript), None, None, None).unwrap(),
            expected,
        );
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
            super::super::claude::parse(
                Cursor::new(transcript),
                Some(&parent),
                None,
                Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
            )
            .unwrap(),
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
            super::super::claude::parse(Cursor::new(transcript), None, None, None).unwrap(),
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
            super::super::claude::parse(Cursor::new(transcript), None, None, None).unwrap(),
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

    fn queued_user(text: &str) -> ChatItem {
        ChatItem::QueuedUser {
            content: vec![UserContentPart::Text {
                text: text.to_owned(),
            }],
            from: None,
        }
    }
}
