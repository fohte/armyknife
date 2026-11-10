use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MessageKind {
    User {
        text: String,
    },
    PeerIn {
        session_id: String,
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
    System,
    Hook,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PeerOut {
    pub(super) session_id: String,
    pub(super) text: String,
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

    if let Some((tag, body)) = tagged_body(text) {
        if is_hidden_tag(tag) {
            return MessageKind::Hidden;
        }
        if let Some((session_id, peer_body)) = peer_message_parts(text) {
            if let Some(user_body) = marked_user_body(peer_body, user_marker) {
                return MessageKind::User { text: user_body };
            }
            return MessageKind::PeerIn {
                session_id: session_id.to_owned(),
                text: peer_body.to_owned(),
            };
        }
        if let Some(user_body) = marked_user_body(text, user_marker) {
            return MessageKind::User { text: user_body };
        }
        return event(tag, body, record);
    }

    if let Some((session_id, peer_body)) = peer_message_parts(text) {
        if let Some(user_body) = marked_user_body(peer_body, user_marker) {
            return MessageKind::User { text: user_body };
        }
        return MessageKind::PeerIn {
            session_id: session_id.to_owned(),
            text: peer_body.to_owned(),
        };
    }

    if let Some(user_body) = marked_user_body(text, user_marker) {
        return MessageKind::User { text: user_body };
    }

    if record.get("type").and_then(Value::as_str) == Some("system") {
        return system_record(record, text);
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
            _ => return MessageKind::Hidden,
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
    body.rsplit_once(concat!("\n", "\n", "This came from another Claude session"))
        .map_or(body, |(body, _)| body)
}

pub(super) fn peer_out_from_tool(name: &str, input: Option<&Value>) -> Option<PeerOut> {
    if name != "Bash" {
        return None;
    }
    let command = input?
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| input.and_then(Value::as_str))?;
    let arguments = shlex::split(command)?;
    let command_start = arguments
        .windows(4)
        .position(|parts| parts == ["a", "agent", "peer", "notify"])?;
    let mut message = None;
    let mut session_id = None;
    let mut index = command_start + 4;
    while index < arguments.len() {
        let argument = &arguments[index];
        if argument == "--message" {
            message = arguments.get(index + 1).cloned();
            index += 2;
            continue;
        }
        if let Some(value) = argument.strip_prefix("--message=") {
            message = Some(value.to_owned());
        } else if !argument.starts_with('-') {
            session_id = Some(argument.clone());
        }
        index += 1;
    }
    Some(PeerOut {
        session_id: session_id?,
        text: message?,
    })
}

fn system_record(record: &Value, raw_text: &str) -> MessageKind {
    let subtype = record
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or("system event");
    let text = match subtype {
        "compact_boundary" => "Context compacted".to_owned(),
        "resume" | "session_resume" => "Session resumed".to_owned(),
        "model_change" | "model_changed" => "Model changed".to_owned(),
        _ if !raw_text.trim().is_empty() => one_line(raw_text),
        _ => subtype.replace('_', " "),
    };
    MessageKind::Event {
        kind: EventKind::System,
        failed: is_failure(record, subtype, &text),
        text,
    }
}

fn event(tag: &str, body: &str, record: &Value) -> MessageKind {
    let kind = if tag == "local-command-stdout" || tag.starts_with("hook-") {
        EventKind::Hook
    } else if tag == "delegation-update" || tag.starts_with("armyknife-") {
        EventKind::Armyknife
    } else {
        EventKind::System
    };
    let text = one_line(body);
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

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tagged_body(text: &str) -> Option<(&str, &str)> {
    let tag = outer_tag(text)?;
    let trimmed = text.trim_start();
    let opening = format!("<{tag}>");
    let closing = format!("</{tag}>");
    let body = trimmed
        .strip_prefix(&opening)?
        .strip_suffix(&closing)?
        .trim();
    Some((tag, body))
}

fn outer_tag(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
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

fn marked_user_body(text: &str, marker: Option<&str>) -> Option<String> {
    let marker = marker?;
    if let Some(body) = crate::commands::agent::watch_web_user_marker::strip_message(marker, text) {
        return Some(body.to_owned());
    }
    if let Some((_, body)) = peer_message_parts(text)
        && let Some(body) =
            crate::commands::agent::watch_web_user_marker::strip_message(marker, body)
    {
        return Some(body.to_owned());
    }
    let marker_offset = text.match_indices(marker).find_map(|(offset, _)| {
        let prefix = &text[..offset];
        (offset == 0 || prefix.ends_with('\n')).then_some(offset)
    })?;
    let body = text[marker_offset + marker.len()..].strip_prefix('\n')?;
    Some(
        body.rsplit_once(concat!("\n", "\n", "This came from another Claude session"))
            .map_or(body, |(body, _)| body)
            .to_owned(),
    )
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

    use super::{EventKind, MessageKind, classify_inbound, peer_out_from_tool};

    #[rstest]
    #[case::human_user(json!({"type":"user","origin":{"kind":"human"}}), "Please continue.", Some("fixture-marker"), MessageKind::User { text: "Please continue.".to_owned() })]
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
    #[case::delegation_event(json!({"type":"user","isMeta":true}), "<delegation-update>Worker completed its fictional task.</delegation-update>", None, MessageKind::Event { kind: EventKind::Armyknife, text: "Worker completed its fictional task.".to_owned(), failed: false })]
    #[case::unknown_tag_is_system(json!({"type":"user","isMeta":true}), "<future-notice>Future notice text.</future-notice>", None, MessageKind::Event { kind: EventKind::System, text: "Future notice text.".to_owned(), failed: false })]
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
                &json!({"type":"system","subtype":"task_failed","exit_code":1}),
                "",
                None,
            ),
            MessageKind::Event {
                kind: EventKind::System,
                text: "task failed".to_owned(),
                failed: true,
            },
        );
    }

    #[test]
    fn classifies_peer_notify_bash_input() {
        assert_eq!(
            peer_out_from_tool(
                "Bash",
                Some(
                    &json!({"command":"a agent peer notify --message 'Please inspect this fictional task.' fixture-target-id"})
                ),
            ),
            Some(super::PeerOut {
                session_id: "fixture-target-id".to_owned(),
                text: "Please inspect this fictional task.".to_owned(),
            }),
        );
    }
}
