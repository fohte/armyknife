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

pub(super) fn is_peer_notify_tool(name: &str, input: Option<&Value>) -> bool {
    peer_notify_arguments(name, input).is_some()
}

pub(super) fn peer_out_from_tool(
    name: &str,
    input: Option<&Value>,
    parent_session_id: Option<&str>,
    result: Option<&str>,
) -> Option<PeerOut> {
    let arguments = peer_notify_arguments(name, input)?;
    let mut message = None;
    let mut session_id = None;
    let mut parent = false;
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if matches!(argument.as_str(), "--message" | "-m") {
            message = arguments.get(index + 1).cloned();
            index += 2;
            continue;
        }
        if let Some(value) = argument.strip_prefix("--message=") {
            message = Some(value.to_owned());
        } else if argument == "--parent" {
            parent = true;
        } else if argument == "--" {
            session_id = arguments.get(index + 1).cloned();
            break;
        } else if !argument.starts_with('-') {
            session_id = Some(argument.clone());
        }
        index += 1;
    }
    Some(PeerOut {
        session_id: result
            .and_then(peer_notify_result_session_id)
            .map(str::to_owned)
            .or_else(|| {
                if parent {
                    parent_session_id.map(str::to_owned)
                } else {
                    session_id.filter(|id| is_literal_session_id(id))
                }
            })
            .unwrap_or_else(|| "unknown".to_owned()),
        text: message?,
    })
}

fn peer_notify_result_session_id(text: &str) -> Option<&str> {
    const PREFIXES: [&str; 3] = [
        "Delivered to session ",
        "Injected into the running turn of Codex session ",
        "Started a new turn in idle Codex session ",
    ];

    text.lines().find_map(|line| {
        let line = line.trim();
        PREFIXES.iter().find_map(|prefix| {
            line.strip_prefix(prefix)
                .and_then(|value| value.strip_suffix('.'))
                .filter(|id| is_literal_session_id(id))
        })
    })
}

fn is_literal_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn peer_notify_arguments(name: &str, input: Option<&Value>) -> Option<Vec<String>> {
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
    Some(arguments.into_iter().skip(command_start + 4).collect())
}

pub(super) fn failed_peer_notification(text: &str) -> MessageKind {
    let text = one_line(text);
    MessageKind::Event {
        kind: EventKind::Armyknife,
        text: if text.is_empty() {
            "Peer notification failed".to_owned()
        } else {
            text
        },
        failed: true,
    }
}

pub(super) fn peer_notification_failed(record: &Value, text: &str) -> bool {
    is_failure(record, "peer-notify", text)
}

fn system_record(record: &Value, raw_text: &str) -> MessageKind {
    if record.get("subtype").and_then(Value::as_str).is_none() && raw_text.trim().is_empty() {
        return MessageKind::Hidden;
    }
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
    let text = if tag == "task-notification" {
        match (
            nested_tag_value(body, "task-id"),
            nested_tag_value(body, "status"),
        ) {
            (Some(task_id), Some(status)) => format!("Background task {task_id} {status}"),
            _ => one_line(body),
        }
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

fn one_line(text: &str) -> String {
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

    use super::{EventKind, MessageKind, classify_inbound, peer_out_from_tool};

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

    #[rstest]
    #[case::long_flag(
        "a agent peer notify --message 'Please inspect this fictional task.' fixture-target-id",
        Some("fixture-target-id")
    )]
    #[case::short_flag(
        "a agent peer notify -m 'Please inspect this fictional task.' fixture-target-id",
        Some("fixture-target-id")
    )]
    #[case::parent(
        "a agent peer notify --parent -m 'Please inspect this fictional task.'",
        Some("fixture-parent-id")
    )]
    fn classifies_peer_notify_bash_input(
        #[case] command: &str,
        #[case] parent_session_id: Option<&str>,
    ) {
        assert_eq!(
            peer_out_from_tool(
                "Bash",
                Some(&json!({"command":command})),
                parent_session_id,
                None,
            ),
            Some(super::PeerOut {
                session_id: if command.contains("--parent") {
                    "fixture-parent-id".to_owned()
                } else {
                    "fixture-target-id".to_owned()
                },
                text: "Please inspect this fictional task.".to_owned(),
            }),
        );
    }

    #[rstest]
    #[case::shell_variable("a agent peer notify -m 'Please inspect this fictional task.' $sid")]
    #[case::command_substitution(
        "a agent peer notify -m 'Please inspect this fictional task.' $(resolve_target)"
    )]
    #[case::backtick_substitution(
        "a agent peer notify -m 'Please inspect this fictional task.' `resolve_target`"
    )]
    fn does_not_treat_shell_expansions_as_session_ids(#[case] command: &str) {
        assert_eq!(
            peer_out_from_tool("Bash", Some(&json!({"command":command})), None, None,),
            Some(super::PeerOut {
                session_id: "unknown".to_owned(),
                text: "Please inspect this fictional task.".to_owned(),
            }),
        );
    }

    #[rstest]
    #[case::current_output("Delivered to session fixture-result-target.")]
    #[case::legacy_codex_output(
        "Injected into the running turn of Codex session fixture-result-target."
    )]
    fn prefers_session_id_from_tool_result(#[case] result: &str) {
        assert_eq!(
            peer_out_from_tool(
                "Bash",
                Some(
                    &json!({"command":"a agent peer notify -m 'Inspect the fictional result.' fixture-argument-target"})
                ),
                None,
                Some(result),
            ),
            Some(super::PeerOut {
                session_id: "fixture-result-target".to_owned(),
                text: "Inspect the fictional result.".to_owned(),
            }),
        );
    }

    #[test]
    fn shows_unknown_when_shell_target_has_no_resolved_result() {
        assert_eq!(
            peer_out_from_tool(
                "Bash",
                Some(
                    &json!({"command":"a agent peer notify --message 'Inspect the fictional result.' $sid"})
                ),
                None,
                Some("delivered"),
            ),
            Some(super::PeerOut {
                session_id: "unknown".to_owned(),
                text: "Inspect the fictional result.".to_owned(),
            }),
        );
    }
}
