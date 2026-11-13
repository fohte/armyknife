use serde_json::Value;

mod system_record;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MessageKind {
    User {
        text: String,
    },
    PeerIn {
        session_id: String,
        text: String,
    },
    SubagentReport {
        agent_id: String,
        agent_name: String,
        text: String,
    },
    Event {
        kind: EventKind,
        text: String,
        failed: bool,
    },
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EventKind {
    Armyknife,
    CompactBoundary,
    System,
    Hook,
}

pub(super) fn classify_inbound(
    record: &Value,
    text: &str,
    user_marker: Option<&str>,
) -> MessageKind {
    if record.get("sourceToolAssistantUUID").is_some()
        || record
            .get("toolUseResult")
            .is_some_and(|result| !result.is_null())
    {
        return MessageKind::Hidden;
    }

    let tagged = tagged_body(text);
    if tagged.is_some_and(|(tag, _)| is_hidden_tag(tag)) {
        return MessageKind::Hidden;
    }
    if let Some(report) = subagent_report(record) {
        return report;
    }
    if origin(record)
        .and_then(|origin| origin.get("kind"))
        .and_then(Value::as_str)
        == Some("human")
    {
        return MessageKind::User {
            text: marked_user_body(text, user_marker).unwrap_or_else(|| text.to_owned()),
        };
    }

    if let Some((tag, body)) = tagged {
        if let Some((session_id, peer_body)) = peer_message_parts(text) {
            if let Some(message) = marked_user_message(peer_body, user_marker) {
                return message;
            }
            return MessageKind::PeerIn {
                session_id: session_id.to_owned(),
                text: peer_body.to_owned(),
            };
        }
        if let Some(message) = marked_user_message(text, user_marker) {
            return message;
        }
        return event(tag, body, record);
    }

    if let Some((session_id, peer_body)) = peer_message_parts(text) {
        if let Some(message) = marked_user_message(peer_body, user_marker) {
            return message;
        }
        return MessageKind::PeerIn {
            session_id: session_id.to_owned(),
            text: peer_body.to_owned(),
        };
    }

    if let Some(message) = marked_user_message(text, user_marker) {
        return message;
    }

    if record.get("type").and_then(Value::as_str) == Some("system") {
        return system_record::classify(record, text);
    }

    if let Some(origin_kind) = origin(record)
        .and_then(|origin| origin.get("kind"))
        .and_then(Value::as_str)
    {
        let event_kind = match origin_kind {
            "armyknife" | "delegation-update" => Some(EventKind::Armyknife),
            "system" | "task-notification" => Some(EventKind::System),
            "hook" => Some(EventKind::Hook),
            "human" | "peer" => None,
            _ => Some(EventKind::System),
        };
        if let Some(kind) = event_kind {
            let text = one_line(text);
            return MessageKind::Event {
                kind,
                failed: is_failure(record, origin_kind, &text),
                text,
            };
        }
    }

    if origin(record)
        .and_then(|origin| origin.get("kind"))
        .and_then(Value::as_str)
        == Some("peer")
    {
        let session_id = origin(record)
            .and_then(|origin| origin.get("from"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        return MessageKind::PeerIn {
            session_id,
            text: peer_transport_body(text).to_owned(),
        };
    }

    if is_hidden_legacy_row(record, text) {
        return MessageKind::Hidden;
    }

    MessageKind::User {
        text: text.to_owned(),
    }
}

fn subagent_report(record: &Value) -> Option<MessageKind> {
    let origin = origin(record)?;
    if origin.get("kind").and_then(Value::as_str) != Some("peer")
        || origin.get("handback").and_then(Value::as_bool) != Some(true)
    {
        return None;
    }
    let agent_id = origin
        .get("from")
        .or_else(|| origin.get("senderTaskId"))
        .and_then(Value::as_str)?
        .to_owned();
    let agent_name = origin
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Subagent")
        .to_owned();
    let text = origin
        .get("handbackReportText")
        .and_then(Value::as_str)?
        .to_owned();
    Some(MessageKind::SubagentReport {
        agent_id,
        agent_name,
        text,
    })
}

fn origin(record: &Value) -> Option<&Value> {
    record
        .get("origin")
        .filter(|origin| !origin.is_null())
        .or_else(|| {
            record
                .get("attachment")
                .and_then(|attachment| attachment.get("origin"))
        })
        .filter(|origin| !origin.is_null())
}

fn peer_transport_body(text: &str) -> &str {
    let body = text
        .strip_prefix("Another peer session sent a message:\n")
        .unwrap_or(text);
    without_peer_transport_notice(body)
}

fn event(tag: &str, body: &str, record: &Value) -> MessageKind {
    let kind = if tag == "local-command-stdout" || tag.starts_with("hook-") {
        EventKind::Hook
    } else if tag == "delegation-update" || tag.starts_with("armyknife-") {
        EventKind::Armyknife
    } else {
        EventKind::System
    };
    let text = if tag == "task-notification" {
        nested_tag_value(body, "summary")
            .filter(|summary| !summary.trim().is_empty())
            .map(one_line)
            .or_else(|| {
                match (
                    nested_tag_value(body, "task-id"),
                    nested_tag_value(body, "status"),
                ) {
                    (Some(task_id), Some(status)) => {
                        Some(format!("Background task {task_id} {status}"))
                    }
                    _ => None,
                }
            })
            .unwrap_or_else(|| one_line(body))
    } else {
        one_line(body)
    };
    MessageKind::Event {
        kind,
        failed: is_failure(record, tag, &text),
        text,
    }
}

fn is_failure(record: &Value, label: &str, text: &str) -> bool {
    ["is_error", "isError", "failed", "blocked"]
        .into_iter()
        .any(|key| record.get(key).and_then(Value::as_bool) == Some(true))
        || record
            .get("exit_code")
            .and_then(Value::as_i64)
            .is_some_and(|exit_code| exit_code != 0)
        || [label, text].into_iter().any(|value| {
            value
                .split(|character: char| !character.is_ascii_alphanumeric())
                .any(|word| {
                    matches!(
                        word.to_ascii_lowercase().as_str(),
                        "fail" | "failed" | "failure" | "blocked"
                    )
                })
        })
}

pub(super) fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn nested_tag_value<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let opening = format!("<{tag}>");
    let closing = format!("</{tag}>");
    text.split_once(&opening)?
        .1
        .split_once(&closing)
        .map(|(value, _)| value)
}

pub(super) fn tagged_body(text: &str) -> Option<(&str, &str)> {
    let tag = outer_tag(text)?;
    let trimmed = text.trim();
    let opening = format!("<{tag}>");
    let closing = format!("</{tag}>");
    let body = trimmed
        .strip_prefix(&opening)?
        .strip_suffix(&closing)?
        .trim();
    Some((tag, body))
}

fn outer_tag(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let tag = trimmed.strip_prefix('<')?.split_once('>')?.0;
    if tag.is_empty()
        || tag.starts_with('/')
        || !tag
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return None;
    }
    trimmed.ends_with(&format!("</{tag}>")).then_some(tag)
}

fn is_hidden_tag(tag: &str) -> bool {
    matches!(tag, "system-reminder" | "local-command-caveat")
}

fn is_hidden_legacy_row(record: &Value, text: &str) -> bool {
    if origin(record)
        .and_then(|origin| origin.get("kind"))
        .is_some()
    {
        return false;
    }
    ["isMeta", "isSidechain", "isCompactSummary"]
        .into_iter()
        .any(|key| {
            record.get(key).and_then(Value::as_bool) == Some(true)
                || record
                    .get("attachment")
                    .and_then(|attachment| attachment.get(key))
                    .and_then(Value::as_bool)
                    == Some(true)
        })
        || (text.trim_start().starts_with("<command-message>") && !text.contains("<command-name>"))
}

pub(super) fn marked_user_body(text: &str, marker: Option<&str>) -> Option<String> {
    let marker = marker?;
    if let Some(body) = crate::commands::agent::watch_web_user_marker::strip_message(marker, text) {
        return Some(without_peer_transport_notice(body).to_owned());
    }
    if let Some((_, body)) = peer_message_parts(text)
        && let Some(body) =
            crate::commands::agent::watch_web_user_marker::strip_message(marker, body)
    {
        return Some(without_peer_transport_notice(body).to_owned());
    }
    let marker_offset = text.match_indices(marker).find_map(|(offset, _)| {
        let prefix = &text[..offset];
        (offset == 0 || prefix.ends_with('\n')).then_some(offset)
    })?;
    let marked_text = &text[marker_offset..];
    if let Some(body) =
        crate::commands::agent::watch_web_user_marker::strip_message(marker, marked_text)
    {
        return Some(without_peer_transport_notice(body).to_owned());
    }
    let body = marked_text.strip_prefix(marker)?.strip_prefix('\n')?;
    Some(without_peer_transport_notice(body).to_owned())
}

fn marked_user_message(text: &str, marker: Option<&str>) -> Option<MessageKind> {
    let body = marked_user_body(text, marker)?;
    match tagged_body(&body) {
        Some((tag, event_body)) if is_watch_web_action_tag(tag) => Some(MessageKind::Event {
            kind: EventKind::Armyknife,
            text: one_line(event_body),
            failed: false,
        }),
        _ => Some(MessageKind::User { text: body }),
    }
}

pub(super) fn is_watch_web_action_tag(tag: &str) -> bool {
    matches!(
        tag,
        "armyknife-todo-completed" | "armyknife-decision-change"
    )
}

fn without_peer_transport_notice(body: &str) -> &str {
    body.rsplit_once(concat!("\n", "\n", "This came from another Claude session"))
        .map_or(body, |(body, _)| body)
}

fn peer_message_parts(message: &str) -> Option<(&str, &str)> {
    const END: &str = "\n</peer-message>";
    let envelope = message.split_once("<peer-message>")?.1;
    let end = envelope.rfind(END)?;
    let envelope = envelope[..end].strip_prefix('\n')?;
    let (header, body) = envelope.split_once(concat!("\n", "\n"))?;
    let mut header_lines = header.lines();
    let sender_id = header_lines.next()?.strip_prefix("- From session_id: ")?;
    if sender_id.is_empty()
        || header_lines
            .next()
            .is_some_and(|line| !line.starts_with("- Sender engine: "))
        || header_lines.next().is_some()
    {
        return None;
    }
    Some((sender_id, body))
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::rstest;
    use serde_json::json;

    use super::{EventKind, MessageKind, classify_inbound};

    const HUMAN_PEER_ENVELOPE: &str = indoc! {r#"
        <peer-message>
        - From session_id: fixture-peer-id

        Human supplied text.
        </peer-message>
    "#};

    #[rstest]
    #[case::human_user(json!({"type":"user","origin":{"kind":"human"}}), "Please continue.", Some("fixture-marker"), MessageKind::User { text: "Please continue.".to_owned() })]
    #[case::human_tagged_text_stays_user(json!({"type":"user","origin":{"kind":"human"}}), "<task-notification>Human supplied text.</task-notification>", None, MessageKind::User { text: "<task-notification>Human supplied text.</task-notification>".to_owned() })]
    #[case::human_peer_envelope_stays_user(json!({"type":"user","origin":{"kind":"human"}}), HUMAN_PEER_ENVELOPE, None, MessageKind::User { text: HUMAN_PEER_ENVELOPE.to_owned() })]
    #[case::unrelated_peer(
        json!({"type":"user","origin":{"kind":"peer","from":"fixture-peer-id"}}),
        indoc! {r#"
            <peer-message>
            - From session_id: fixture-peer-id

            Continue the fictional task.
            </peer-message>
        "#},
        None,
        MessageKind::PeerIn { session_id: "fixture-peer-id".to_owned(), text: "Continue the fictional task.".to_owned() }
    )]
    #[case::subagent_handback_uses_report_text(
        json!({"type":"user","origin":{"kind":"peer","from":"fixture-agent-id","name":"general-purpose","handback":true,"handbackReportText":"The fictional task is complete."}}),
        "Another Claude session sent a message:\n[Subagent hand-back] boilerplate",
        None,
        MessageKind::SubagentReport {
            agent_id: "fixture-agent-id".to_owned(),
            agent_name: "general-purpose".to_owned(),
            text: "The fictional task is complete.".to_owned(),
        }
    )]
    #[case::delegation_event(json!({"type":"user","isMeta":true}), "<delegation-update>Worker completed its fictional task.</delegation-update>", None, MessageKind::Event { kind: EventKind::Armyknife, text: "Worker completed its fictional task.".to_owned(), failed: false })]
    #[case::marked_armyknife_action_is_an_event(
        json!({"type":"user","isMeta":true,"origin":{"kind":"peer","from":"unknown"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-cobalt⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            <armyknife-todo-completed>Investigate failed fixture tasks.

            A failed attempt is still informative.</armyknife-todo-completed>
        "},
        Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
        MessageKind::Event { kind: EventKind::Armyknife, text: "Investigate failed fixture tasks. A failed attempt is still informative.".to_owned(), failed: false },
    )]
    #[case::marked_decision_change_is_an_event(
        json!({"type":"user","origin":{"kind":"peer","from":"unknown"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-cobalt⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            <armyknife-decision-change>Change requested for the fixture decision.

            Please use the second fictional option.</armyknife-decision-change>
        "},
        Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
        MessageKind::Event { kind: EventKind::Armyknife, text: "Change requested for the fixture decision. Please use the second fictional option.".to_owned(), failed: false },
    )]
    #[case::marked_unrecognized_armyknife_tag_stays_user_text(
        json!({"type":"user","origin":{"kind":"peer","from":"unknown"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-cobalt⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            <armyknife-example>First line.
            Second line.</armyknife-example>
        "},
        Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
        MessageKind::User {
            text: indoc! {"
                <armyknife-example>First line.
                Second line.</armyknife-example>
            "}
            .to_owned(),
        },
    )]
    #[case::unknown_tag_is_system(json!({"type":"user","isMeta":true}), "<future-notice>Future notice text.</future-notice>", None, MessageKind::Event { kind: EventKind::System, text: "Future notice text.".to_owned(), failed: false })]
    #[case::compact_boundary_has_a_distinct_kind(json!({"type":"system","subtype":"compact_boundary"}), "", None, MessageKind::Event { kind: EventKind::CompactBoundary, text: "Context compacted".to_owned(), failed: false })]
    #[case::background_task_uses_summary(json!({"type":"user"}), "<task-notification><task-id>fixture-task-id</task-id><status>completed</status><summary>Background command \"Check the fictional build\" completed (exit code 0)</summary></task-notification>", None, MessageKind::Event { kind: EventKind::System, text: "Background command \"Check the fictional build\" completed (exit code 0)".to_owned(), failed: false })]
    #[case::background_task_without_summary_uses_id_and_status(json!({"type":"user"}), "<task-notification><task-id>fixture-task-id</task-id><status>completed</status></task-notification>", None, MessageKind::Event { kind: EventKind::System, text: "Background task fixture-task-id completed".to_owned(), failed: false })]
    #[case::away_summary_uses_record_content(json!({"type":"system","subtype":"away_summary","content":"The fictional task finished while the user was away."}), "", None, MessageKind::Event { kind: EventKind::System, text: "recap: The fictional task finished while the user was away.".to_owned(), failed: false })]
    #[case::away_summary_uses_raw_text_when_content_is_empty(json!({"type":"system","subtype":"away_summary","content":"  "}), "The fictional task finished while the user was away.", None, MessageKind::Event { kind: EventKind::System, text: "recap: The fictional task finished while the user was away.".to_owned(), failed: false })]
    #[case::empty_stop_hook_summary_is_hidden(json!({"type":"system","subtype":"stop_hook_summary","hookErrors":[],"preventedContinuation":false}), "", None, MessageKind::Hidden)]
    #[case::stop_hook_error_is_visible_and_marked_failed(json!({"type":"system","subtype":"stop_hook_summary","hookErrors":[{"hookName":"fixture-hook","error":"Synthetic hook failure."}],"preventedContinuation":false}), "", None, MessageKind::Event { kind: EventKind::System, text: "Stop hook summary: fixture-hook: Synthetic hook failure.".to_owned(), failed: true })]
    #[case::prevented_continuation_is_visible(json!({"type":"system","subtype":"stop_hook_summary","hookErrors":[],"preventedContinuation":true}), "", None, MessageKind::Event { kind: EventKind::System, text: "Stop hook summary: continuation was prevented".to_owned(), failed: false })]
    #[case::turn_duration_shows_elapsed_time_only(json!({"type":"system","subtype":"turn_duration","durationMs":30314,"messageCount":5,"timestamp":"2026-06-01T12:34:00Z"}), "", None, MessageKind::Event { kind: EventKind::System, text: "Took 30s".to_owned(), failed: false })]
    #[case::hook_output_is_one_line(json!({"type":"user"}), "<local-command-stdout>First line.\nSecond line.</local-command-stdout>", None, MessageKind::Event { kind: EventKind::Hook, text: "First line. Second line.".to_owned(), failed: false })]
    fn classifies_inbound_records(
        #[case] record: serde_json::Value,
        #[case] text: &str,
        #[case] marker: Option<&str>,
        #[case] expected: MessageKind,
    ) {
        assert_eq!(classify_inbound(&record, text, marker), expected);
    }

    #[test]
    fn retains_hidden_system_reminders() {
        assert_eq!(
            classify_inbound(
                &json!({"type":"user"}),
                "<system-reminder>Internal context.</system-reminder>",
                None,
            ),
            MessageKind::Hidden,
        );
    }

    #[test]
    fn marks_failed_events() {
        assert_eq!(
            classify_inbound(
                &json!({"type":"system","subtype":"task","exit_code":1}),
                "",
                None,
            ),
            MessageKind::Event {
                kind: EventKind::System,
                text: "task".to_owned(),
                failed: true,
            },
        );
    }
}
