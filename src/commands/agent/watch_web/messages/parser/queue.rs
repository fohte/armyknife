use std::collections::VecDeque;
use std::io::{self, BufRead, Seek, SeekFrom};

use serde_json::Value;

use super::super::{ChatItem, ParentSession, claude_content_text};
use super::{append_text_with_user_marker, for_each_jsonl_record};

pub(super) fn parse(
    reader: &mut (impl BufRead + Seek),
    parent: Option<&ParentSession>,
    user_marker: Option<&str>,
) -> io::Result<Vec<ChatItem>> {
    reader.seek(SeekFrom::Start(0))?;
    let mut queued = VecDeque::new();
    let mut recently_removed_prompt = None::<Option<String>>;
    for_each_jsonl_record(reader, |record| {
        if record.get("type").and_then(Value::as_str) == Some("queue-operation") {
            match record.get("operation").and_then(Value::as_str) {
                Some("enqueue") => {
                    recently_removed_prompt = None;
                    queued.push_back(
                        record
                            .get("content")
                            .filter(|content| !content.is_null())
                            .map(|content| claude_content_text(Some(content))),
                    );
                }
                Some("dequeue") => {
                    recently_removed_prompt = queued.pop_front();
                }
                Some("remove") => {
                    recently_removed_prompt = None;
                    let content = record
                        .get("content")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let index = content
                        .as_ref()
                        .and_then(|content| matching_prompt_index(&queued, content));
                    let index = index.or_else(|| {
                        content
                            .as_ref()
                            .and_then(|_| queued.iter().position(Option::is_none))
                    });
                    if let Some(index) = index {
                        let removed = queued.remove(index).flatten();
                        recently_removed_prompt = Some(removed.or(content));
                    }
                }
                Some("popAll") => {
                    queued.clear();
                    recently_removed_prompt = None;
                }
                _ => recently_removed_prompt = None,
            }
            return;
        }

        if record
            .get("attachment")
            .and_then(|attachment| attachment.get("type"))
            .and_then(Value::as_str)
            == Some("queued_command")
            && let Some(prompt) = record
                .get("attachment")
                .and_then(|attachment| attachment.get("prompt"))
        {
            let prompt = claude_content_text(Some(prompt));
            let already_removed = recently_removed_prompt
                .as_ref()
                .is_some_and(|removed| removed.as_deref().is_none_or(|text| text == prompt));
            if !already_removed && let Some(index) = matching_prompt_index(&queued, &prompt) {
                queued.remove(index);
            }
        }
        recently_removed_prompt = None;
    })?;

    let mut items = Vec::new();
    for prompt in queued.into_iter().flatten() {
        let start = items.len();
        append_text_with_user_marker(&mut items, prompt, parent, user_marker);
        if items.len() > start
            && let Some(ChatItem::User { text, from }) = items.pop()
        {
            items.push(ChatItem::QueuedUser { text, from });
        }
    }
    Ok(items)
}

fn matching_prompt_index(queued: &VecDeque<Option<String>>, prompt: &str) -> Option<usize> {
    queued
        .iter()
        .position(|queued_prompt| queued_prompt.as_deref() == Some(prompt))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use indoc::indoc;
    use rstest::rstest;

    use super::parse;
    use crate::commands::agent::watch_web::messages::{ChatItem, ParentSession};

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
            parse(&mut Cursor::new(transcript), None, None).unwrap(),
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
            parse(
                &mut Cursor::new(transcript),
                Some(&parent),
                Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
            )
            .unwrap(),
            vec![ChatItem::QueuedUser {
                text: "Queued user request.".to_string(),
                from: None,
            }],
        );
    }

    #[test]
    fn delivered_attachment_does_not_remove_a_duplicate_queued_prompt() {
        let transcript = indoc! {r#"
            {"type":"queue-operation","operation":"enqueue","content":"Repeated prompt."}
            {"type":"queue-operation","operation":"enqueue","content":"Repeated prompt."}
            {"type":"queue-operation","operation":"remove","reason":"absorbed_mid_turn","content":"Repeated prompt."}
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
            text: text.to_owned(),
            from: None,
        }
    }
}
