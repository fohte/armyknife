use serde_json::Value;

use crate::commands::agent::peer_message::{
    ExtractedPeerMessage, extract_peer_message, task_notification_envelope,
};

use super::super::action_event;
use super::super::tagged_message::{tagged_action_body, tagged_body};

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

    let content_without_id = super::super::queued_prompt::strip_id_marker(text);
    let tagged = tagged_body(text).or_else(|| tagged_action_body(&content_without_id));
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
        return match marked_user_body(text, user_marker) {
            Some(body) => marked_user_message_body(user_text_without_session_context(&body)),
            None => MessageKind::User {
                text: user_text_without_session_context(text),
            },
        };
    }

    if let Some(message) = task_notification_reminder(record, text) {
        return message;
    }

    if let Some(message) = marked_or_peer_message(text, user_marker) {
        return message;
    }

    if let Some((tag, body)) = tagged {
        if action_event::is_action_event_tag(tag) {
            return action_event::event_text(
                tag,
                body,
                super::super::queued_prompt::id_from_content(text),
            )
            .map_or_else(
                || MessageKind::User {
                    text: text.to_owned(),
                },
                |text| MessageKind::Event {
                    kind: EventKind::Armyknife,
                    text,
                    failed: false,
                },
            );
        }
        return event(tag, body, record);
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
                failed: is_failure(record),
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
            text: peer_transport_body(text),
        };
    }

    if is_hidden_legacy_row(record, text) {
        return MessageKind::Hidden;
    }

    MessageKind::User {
        text: text.to_owned(),
    }
}

pub(super) fn user_text_without_session_context(text: &str) -> String {
    let Some(context_end) = text.rfind("</session-context>") else {
        return text.to_owned();
    };
    if !text[context_end + "</session-context>".len()..]
        .trim()
        .is_empty()
    {
        return text.to_owned();
    }
    let Some(context_start) = text[..context_end].rfind("<session-context>") else {
        return text.to_owned();
    };
    let context = &text[context_start + "<session-context>".len()..context_end];
    if !is_generated_session_context(context) {
        return text.to_owned();
    }
    let before_context = &text[..context_start];
    let prompt = before_context
        .strip_suffix('\n')
        .and_then(|before| before.strip_suffix('\n'))
        .or_else(|| {
            before_context
                .strip_suffix('\n')
                .and_then(|before| before.strip_suffix('\r'))
                .and_then(|before| before.strip_suffix('\n'))
                .and_then(|before| before.strip_suffix('\r'))
        });
    prompt.map_or_else(|| text.to_owned(), str::to_owned)
}

fn is_generated_session_context(context: &str) -> bool {
    let lines: Vec<_> = context.trim().lines().collect();
    match lines.as_slice() {
        [working_directory, parent_working_directory] => {
            has_context_field(working_directory, "- Working directory: ")
                && has_context_field(parent_working_directory, "- Parent working directory: ")
        }
        [working_directory, optional_field, parent_working_directory] => {
            has_context_field(working_directory, "- Working directory: ")
                && (has_context_field(optional_field, "- Branch: ")
                    || has_context_field(optional_field, "- Base: "))
                && has_context_field(parent_working_directory, "- Parent working directory: ")
        }
        [working_directory, branch, base, parent_working_directory] => {
            has_context_field(working_directory, "- Working directory: ")
                && has_context_field(branch, "- Branch: ")
                && has_context_field(base, "- Base: ")
                && has_context_field(parent_working_directory, "- Parent working directory: ")
        }
        _ => false,
    }
}

fn has_context_field(line: &str, field: &str) -> bool {
    line.starts_with(field)
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

fn peer_transport_body(text: &str) -> String {
    extract_peer_message(text, None).map_or_else(|| text.to_owned(), |message| message.body)
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
        failed: is_failure(record)
            || (tag == "task-notification" && task_notification_failed(body)),
        text,
    }
}

fn task_notification_failed(notification_body: &str) -> bool {
    nested_tag_value(notification_body, "status") == Some("failed")
}

fn task_notification_reminder(record: &Value, text: &str) -> Option<MessageKind> {
    let origin_kind = origin(record)
        .and_then(|origin| origin.get("kind"))
        .and_then(Value::as_str);
    if origin_kind != Some("task-notification") {
        return None;
    }

    let (notification_body, reminder_body) = task_notification_and_reminder(text)?;
    let summary = nested_tag_value(notification_body, "summary")?.trim();
    if summary.is_empty() {
        return None;
    }

    let reminder_body = strip_stop_hook_error_prefix(reminder_body);
    let text = one_line(&format!("{summary}: {reminder_body}"));
    let failed = has_explicit_failure(record) || task_notification_failed(notification_body);
    Some(MessageKind::Event {
        kind: EventKind::System,
        text,
        failed,
    })
}

fn task_notification_and_reminder(text: &str) -> Option<(&str, &str)> {
    let envelope = task_notification_envelope(text)?;
    let notification_body = envelope
        .notification
        .strip_prefix("<task-notification>")?
        .strip_suffix("</task-notification>")?;
    let (reminder_tag, reminder_body) = tagged_body(envelope.trailing_text.trim())?;
    (reminder_tag == "system-reminder").then_some((notification_body, reminder_body))
}

fn strip_stop_hook_error_prefix(text: &str) -> &str {
    const PREFIX: &str = "Stop hook blocking error from command \"";
    let Some(after_prefix) = text.strip_prefix(PREFIX) else {
        return text;
    };
    after_prefix
        .split_once("\": ")
        .map_or(text, |(_, message)| message)
}

fn is_failure(record: &Value) -> bool {
    has_explicit_failure(record)
}

fn has_explicit_failure(record: &Value) -> bool {
    ["is_error", "isError", "failed", "blocked"]
        .into_iter()
        .any(|key| record.get(key).and_then(Value::as_bool) == Some(true))
        || record
            .get("exit_code")
            .and_then(Value::as_i64)
            .is_some_and(|exit_code| exit_code != 0)
        || record
            .get("hookErrors")
            .and_then(Value::as_array)
            .is_some_and(|errors| !errors.is_empty())
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
    let message = extract_peer_message(text, marker)?;
    message.has_user_marker.then_some(message.body)
}

fn marked_or_peer_message(text: &str, marker: Option<&str>) -> Option<MessageKind> {
    let message = extract_peer_message(text, marker)?;
    if message.has_user_marker_in_body || (message.sender_id.is_none() && message.has_user_marker) {
        return Some(marked_user_message_body(user_text_without_session_context(
            &message.body,
        )));
    }
    let text = peer_message_text(&message);
    let session_id = message.sender_id?;
    Some(MessageKind::PeerIn { session_id, text })
}

fn peer_message_text(message: &ExtractedPeerMessage) -> String {
    message.transport_notice_suffix.as_deref().map_or_else(
        || message.body.clone(),
        |notice| format!("{}{}", message.body, notice),
    )
}

fn marked_user_message_body(body: String) -> MessageKind {
    let queued_prompt_id = super::super::queued_prompt::id_from_content(&body).map(str::to_owned);
    let body_without_id = super::super::queued_prompt::strip_id_marker(&body);
    match tagged_action_body(&body_without_id).and_then(|(tag, event_body)| {
        action_event::event_text(tag, event_body, queued_prompt_id.as_deref())
    }) {
        Some(text) => {
            let text = match queued_prompt_id {
                Some(message_id) => super::super::queued_prompt::add_id_marker(&text, &message_id),
                None => text,
            };
            MessageKind::Event {
                kind: EventKind::Armyknife,
                text,
                failed: false,
            }
        }
        _ => MessageKind::User { text: body },
    }
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

    const TASK_REMINDER_BODY: &str = indoc! {r#"
        <task-notification>
        <summary>Long-running command reminder</summary>
        </task-notification>
        <system-reminder>
        Stop hook blocking error from command "SyntheticHook:Shell": Reminder for the synthetic worker: it is blocked while waiting for the fixture.
        </system-reminder>
    "#};

    #[rstest]
    #[case::human_user(json!({"type":"user","origin":{"kind":"human"}}), "Please continue.", Some("fixture-marker"), MessageKind::User { text: "Please continue.".to_owned() })]
    #[case::human_session_context_is_hidden_and_prompt_text_remains(
        json!({"type":"user","origin":{"kind":"human"}}),
        indoc! {"
            Please inspect this fictional issue.

            <session-context>
            - User-authored note: keep this literal block.
            </session-context>

            Keep this follow-up sentence.

            <session-context>
            - Working directory: /fictional/project
            - Parent working directory: /fictional
            </session-context>
        "},
        None,
        MessageKind::User {
            text: indoc! {"
                Please inspect this fictional issue.

                <session-context>
                - User-authored note: keep this literal block.
                </session-context>

                Keep this follow-up sentence."}
            .to_owned(),
        },
    )]
    #[case::marked_human_session_context_is_hidden_and_prompt_text_remains(
        json!({"type":"user","origin":{"kind":"human"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-violet⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            Please inspect this marked fictional issue.

            <session-context>
            - Working directory: /fictional/project
            - Parent working directory: /fictional
            </session-context>
        "},
        Some("⟦armyknife-user:fixture-marker-violet⟧"),
        MessageKind::User {
            text: "Please inspect this marked fictional issue.".to_owned(),
        },
    )]
    #[case::peer_origin_watch_web_marker_is_user_and_context_is_hidden(
        json!({"type":"user","origin":{"kind":"peer","from":"fixture-sender-id"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-violet⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            Please inspect this peer-delivered fictional issue.

            <session-context>
            - Working directory: /fictional/project
            - Parent working directory: /fictional
            </session-context>
        "},
        Some("⟦armyknife-user:fixture-marker-violet⟧"),
        MessageKind::User {
            text: "Please inspect this peer-delivered fictional issue.".to_owned(),
        },
    )]
    #[case::peer_origin_watch_web_marker_preserves_user_authored_context(
        json!({"type":"user","origin":{"kind":"peer","from":"fixture-sender-id"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-violet⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            Please keep this literal context block.

            <session-context>
            - User-authored note: preserve this text.
            </session-context>
        "},
        Some("⟦armyknife-user:fixture-marker-violet⟧"),
        MessageKind::User {
            text: indoc! {"
                Please keep this literal context block.

                <session-context>
                - User-authored note: preserve this text.
                </session-context>
            "}
            .to_owned(),
        },
    )]
    #[case::human_vibe_notice_is_an_event(
        json!({"type":"user","origin":{"kind":"human"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-violet⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            <armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>
            <!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-teal -->
        "},
        Some("⟦armyknife-user:fixture-marker-violet⟧"),
        MessageKind::Event {
            kind: EventKind::Armyknife,
            text: "Vibe mode turned on\n<!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-teal -->"
                .to_owned(),
            failed: false,
        },
    )]
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
    #[case::peer_context_remains_visible(
        json!({"type":"user","origin":{"kind":"peer","from":"fixture-peer-id"}}),
        indoc! {r#"
            <peer-message>
            - From session_id: fixture-peer-id

            ping

            <session-context>
            - Working directory: /fictional/project
            </session-context>

            This came from another Claude session — synthetic transport notice.
            </peer-message>
        "#},
        None,
        MessageKind::PeerIn {
            session_id: "fixture-peer-id".to_owned(),
            text: indoc! {"
                ping

                <session-context>
                - Working directory: /fictional/project
                </session-context>

                This came from another Claude session — synthetic transport notice.
            "}.trim().to_owned(),
        }
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
            <!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-gold -->
        "},
        Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
        MessageKind::Event { kind: EventKind::Armyknife, text: "Investigate failed fixture tasks. A failed attempt is still informative.\n<!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-gold -->".to_owned(), failed: false },
    )]
    #[case::marked_decision_change_is_an_event(
        json!({"type":"user","origin":{"kind":"peer","from":"unknown"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-cobalt⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            <armyknife-decision-change>Change requested for the fixture decision.

            Please use the second fictional option.</armyknife-decision-change>
            <!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-lime -->
        "},
        Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
        MessageKind::Event { kind: EventKind::Armyknife, text: "Change requested for the fixture decision. Please use the second fictional option.\n<!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-lime -->".to_owned(), failed: false },
    )]
    #[case::marked_todo_rejection_with_attachment_paths_is_an_event(
        json!({"type":"user","origin":{"kind":"peer","from":"unknown"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-cobalt⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            <armyknife-todo-rejected>The user rejected the to-do: Review the fixture.</armyknife-todo-rejected>

            Attached file(s):
            - attachment-store/fixture.gif
            <!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-violet -->
        "},
        Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
        MessageKind::Event {
            kind: EventKind::Armyknife,
            text: "The user rejected the to-do: Review the fixture.\n<!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-violet -->".to_owned(),
            failed: false,
        },
    )]
    #[case::marked_decision_change_with_attachment_paths_is_an_event(
        json!({"type":"user","origin":{"kind":"peer","from":"unknown"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-cobalt⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            <armyknife-decision-change>Change requested for the fixture decision.</armyknife-decision-change>

            Attached file(s):
            - attachment-store/fixture.gif
            <!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-amber -->
        "},
        Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
        MessageKind::Event {
            kind: EventKind::Armyknife,
            text: "Change requested for the fixture decision.\n<!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-amber -->".to_owned(),
            failed: false,
        },
    )]
    #[case::marked_vibe_mode_on_is_an_event(
        json!({"type":"user","origin":{"kind":"peer","from":"unknown"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-cobalt⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            <armyknife-vibe-mode-on>watch-web からユーザーが vibe mode を on にしました。</armyknife-vibe-mode-on>
            <!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-cyan -->
        "},
        Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
        MessageKind::Event { kind: EventKind::Armyknife, text: "Vibe mode turned on\n<!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-cyan -->".to_owned(), failed: false },
    )]
    #[case::marked_vibe_mode_off_is_an_event(
        json!({"type":"user","origin":{"kind":"peer","from":"unknown"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-cobalt⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            <armyknife-vibe-mode-off>watch-web からユーザーが vibe mode を off にしました。祖先セッションから引き継いでいる場合、vibe mode は引き続き有効です。</armyknife-vibe-mode-off>
            <!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-amber -->
        "},
        Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
        MessageKind::Event { kind: EventKind::Armyknife, text: "Vibe mode turned off\n<!-- armyknife-queued-prompt-id:armyknife-action:fixture-message-amber -->".to_owned(), failed: false },
    )]
    #[case::marked_vibe_tag_with_user_text_stays_user(
        json!({"type":"user","origin":{"kind":"peer","from":"unknown"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-cobalt⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            <armyknife-vibe-mode-on>Here is user-authored text.</armyknife-vibe-mode-on>
        "},
        Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
        MessageKind::User {
            text: "<armyknife-vibe-mode-on>Here is user-authored text.</armyknife-vibe-mode-on>\n".to_owned(),
        },
    )]
    #[case::marked_action_tag_with_a_regular_id_stays_user(
        json!({"type":"user","origin":{"kind":"peer","from":"unknown"}}),
        indoc! {"
            ⟦armyknife-user:fixture-marker-cobalt⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            <armyknife-todo-completed>Here is user-authored text.</armyknife-todo-completed>
            <!-- armyknife-queued-prompt-id:fixture-message-rose -->
        "},
        Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
        MessageKind::User {
            text: indoc! {"
                <armyknife-todo-completed>Here is user-authored text.</armyknife-todo-completed>
                <!-- armyknife-queued-prompt-id:fixture-message-rose -->
            "}
            .to_owned(),
        },
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
    #[case::task_reminder_combines_summary_and_reminder_without_failure(
        json!({"type":"user","origin":{"kind":"task-notification","producer":"session-task"},"promptSource":"system","turnOrigin":"task_notification"}),
        TASK_REMINDER_BODY,
        None,
        MessageKind::Event {
            kind: EventKind::System,
            text: "Long-running command reminder: Reminder for the synthetic worker: it is blocked while waiting for the fixture.".to_owned(),
            failed: false,
        }
    )]
    #[case::task_reminder_preserves_explicit_failure(
        json!({"type":"user","origin":{"kind":"task-notification","producer":"session-task"},"is_error":true}),
        TASK_REMINDER_BODY,
        None,
        MessageKind::Event {
            kind: EventKind::System,
            text: "Long-running command reminder: Reminder for the synthetic worker: it is blocked while waiting for the fixture.".to_owned(),
            failed: true,
        }
    )]
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

    #[rstest]
    #[case::nonzero_exit_code(
        json!({"type":"system","subtype":"task","exit_code":1}),
        "",
        MessageKind::Event { kind: EventKind::System, text: "task".to_owned(), failed: true }
    )]
    #[case::explicit_error_flag(
        json!({"type":"user","origin":{"kind":"system"},"isError":true}),
        "",
        MessageKind::Event { kind: EventKind::System, text: "".to_owned(), failed: true }
    )]
    #[case::hook_errors(
        json!({"type":"system","subtype":"stop_hook_summary","hookErrors":[{"error":"Synthetic issue."}]}),
        "",
        MessageKind::Event { kind: EventKind::System, text: "Stop hook summary: Synthetic issue.".to_owned(), failed: true }
    )]
    #[case::failure_words_in_text_are_prose(
        json!({"type":"user","origin":{"kind":"system"}}),
        "This fictional task failed but is not a failure; it is blocked.",
        MessageKind::Event { kind: EventKind::System, text: "This fictional task failed but is not a failure; it is blocked.".to_owned(), failed: false }
    )]
    #[case::failure_words_in_label_are_prose(
        json!({"type":"user","origin":{"kind":"blocked"}}),
        "Synthetic event text.",
        MessageKind::Event { kind: EventKind::System, text: "Synthetic event text.".to_owned(), failed: false }
    )]
    fn marks_failed_events(
        #[case] record: serde_json::Value,
        #[case] text: &str,
        #[case] expected: MessageKind,
    ) {
        assert_eq!(classify_inbound(&record, text, None), expected);
    }
}
