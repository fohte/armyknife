pub(super) const PEER_MESSAGE_START_TAG: &str = "<peer-message>";
pub(super) const PEER_MESSAGE_END_TAG: &str = "</peer-message>";
pub(super) const PEER_MESSAGE_END_MARKER: &str = "\n</peer-message>";
pub(super) const PEER_MESSAGE_SENDER_PREFIX: &str = "<peer-message>\n- From session_id: ";
pub(super) const ANOTHER_CLAUDE_SESSION_MESSAGE_PREFIX: &str =
    "Another Claude session sent a message:";
pub(super) const ANOTHER_PEER_SESSION_MESSAGE_PREFIX: &str =
    "Another peer session sent a message:\n";

pub(super) fn is_peer_message(prompt: Option<&str>) -> bool {
    prompt.is_some_and(|prompt| {
        prompt.starts_with(ANOTHER_CLAUDE_SESSION_MESSAGE_PREFIX)
            || prompt.starts_with(ANOTHER_PEER_SESSION_MESSAGE_PREFIX)
            || prompt.starts_with(PEER_MESSAGE_SENDER_PREFIX)
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ExtractedPeerMessage {
    pub(super) sender_id: Option<String>,
    pub(super) body: String,
    pub(super) cleaned_body: String,
    pub(super) has_user_marker: bool,
}

/// Removes transport details shared by provisional titles and watch-web messages.
pub(super) fn extract_peer_message(
    message: &str,
    marker: Option<&str>,
) -> Option<ExtractedPeerMessage> {
    let mut body = message;
    let mut is_peer_message = false;

    for prefix in [
        ANOTHER_CLAUDE_SESSION_MESSAGE_PREFIX,
        ANOTHER_PEER_SESSION_MESSAGE_PREFIX.trim_end(),
    ] {
        if let Some(rest) = body.strip_prefix(prefix) {
            body = rest.trim_start_matches(['\r', '\n', ' ']);
            is_peer_message = true;
            break;
        }
    }

    let mut sender_id = None;
    if let Some((id, envelope_body)) = peer_message_parts(body) {
        sender_id = Some(id.to_owned());
        body = envelope_body;
        is_peer_message = true;
    }

    let (body, has_user_marker) = strip_watch_web_marker(body, marker);
    let without_notice = without_transport_notice(body);
    let had_transport_notice = without_notice != body;
    let cleaned_body = without_notice.to_owned();
    (is_peer_message || has_user_marker || had_transport_notice).then_some(ExtractedPeerMessage {
        sender_id,
        body: body.to_owned(),
        cleaned_body,
        has_user_marker,
    })
}

pub(super) fn clean_peer_message(message: &str) -> String {
    extract_peer_message(message, None)
        .map_or_else(|| message.trim().to_owned(), |message| message.cleaned_body)
}

pub(super) fn without_session_context(body: &str) -> String {
    let Some(start) = body.find("<session-context>") else {
        return body.trim().to_owned();
    };
    let Some(end) = body[start..].find("</session-context>") else {
        return body.trim().to_owned();
    };
    let after_context = start + end + "</session-context>".len();
    let before = body[..start].trim_end();
    let after = body[after_context..].trim_start();
    match (before.is_empty(), after.is_empty()) {
        (false, false) => {
            let mut result = before.to_owned();
            result.push('\n');
            result.push('\n');
            result.push_str(after);
            result
        }
        (false, true) => before.to_owned(),
        (true, false) => after.to_owned(),
        (true, true) => String::new(),
    }
}

fn strip_watch_web_marker<'a>(body: &'a str, marker: Option<&str>) -> (&'a str, bool) {
    let marker_offset = marker
        .and_then(|marker| {
            body.match_indices(marker).find_map(|(offset, _)| {
                (offset == 0 || body[..offset].ends_with('\n')).then_some((offset, marker))
            })
        })
        .or_else(|| {
            body.lines().enumerate().find_map(|(line_number, line)| {
                (line.starts_with("⟦armyknife-user:") && line.ends_with('⟧')).then_some((
                    body.lines().take(line_number).map(str::len).sum::<usize>() + line_number,
                    line,
                ))
            })
        });
    let Some((offset, marker)) = marker_offset else {
        return (body, false);
    };
    let marked_message = &body[offset..];
    crate::commands::agent::watch_web_user_marker::strip_message(marker, marked_message)
        .map_or_else(
            || {
                (
                    marked_message
                        .strip_prefix(marker)
                        .unwrap_or(marked_message)
                        .trim_start_matches('\n'),
                    true,
                )
            },
            |body| (body, true),
        )
}

fn without_transport_notice(body: &str) -> &str {
    body.rsplit_once(concat!("\n", "\n", "This came from another Claude session"))
        .map_or(body, |(body, _)| body)
}

fn peer_message_parts(message: &str) -> Option<(&str, &str)> {
    let envelope = message.split_once(PEER_MESSAGE_START_TAG)?.1;
    let end = envelope.rfind(PEER_MESSAGE_END_MARKER)?;
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

#[derive(Debug, PartialEq, Eq)]
pub(super) struct TaskNotificationEnvelope<'a> {
    pub(super) notification: &'a str,
    pub(super) trailing_text: &'a str,
}

pub(super) fn task_notification_envelope(text: &str) -> Option<TaskNotificationEnvelope<'_>> {
    let text = text.trim();
    let envelope = if text.starts_with("<task-notification>") {
        text
    } else {
        let (preamble, remainder) = text.split_once('\n')?;
        let preamble = preamble.trim();
        let is_notification_preamble = preamble
            .strip_prefix('[')
            .and_then(|preamble| preamble.strip_suffix(']'))
            .is_some_and(|preamble| preamble.to_ascii_lowercase().contains("notification"));
        if !is_notification_preamble {
            return None;
        }
        remainder.trim_start()
    };
    let body = envelope.strip_prefix("<task-notification>")?;
    let closing_tag = "</task-notification>";
    let end = "<task-notification>".len() + body.find(closing_tag)? + closing_tag.len();
    let trailing_text = &envelope[end..];
    let trailing_text_start = trailing_text.trim_start();
    let check_in_marker = "This is a check-in, not a completion.";
    let is_check_in = trailing_text_start
        .get(..check_in_marker.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(check_in_marker));
    let is_system_reminder = trailing_text_start.starts_with("<system-reminder>");
    if !trailing_text.trim().is_empty() && !is_check_in && !is_system_reminder {
        return None;
    }
    Some(TaskNotificationEnvelope {
        notification: &envelope[..end],
        trailing_text,
    })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{TaskNotificationEnvelope, is_peer_message, task_notification_envelope};

    #[rstest]
    #[case::claude_peer(
        Some(indoc::indoc! {"
            Another Claude session sent a message:
            Please review this.
        "}),
        true
    )]
    #[case::peer(
        Some(indoc::indoc! {"
            Another peer session sent a message:
            Please review this.
        "}),
        true
    )]
    #[case::armyknife_peer(
        Some(indoc::indoc! {"
            <peer-message>
            - From session_id: fixture-peer-id

            Please review this.
            </peer-message>
        "}),
        true
    )]
    #[case::human(Some("Please review this."), false)]
    #[case::human_mentioning_envelope(Some("<peer-message> is part of the prompt."), false)]
    #[case::missing(None, false)]
    fn identifies_peer_prompts(#[case] prompt: Option<&str>, #[case] expected: bool) {
        assert_eq!(is_peer_message(prompt), expected);
    }

    #[rstest]
    #[case::complete_envelope(
        "<task-notification><summary>Fixture check-in</summary></task-notification>",
        Some(TaskNotificationEnvelope {
            notification: "<task-notification><summary>Fixture check-in</summary></task-notification>",
            trailing_text: "",
        }),
    )]
    #[case::envelope_followed_by_guidance(
        indoc::indoc! {"
            <task-notification><summary>Fixture check-in</summary></task-notification>
            This is a check-in, not a completion. The fictional task may be working quietly.
        "},
        Some(TaskNotificationEnvelope {
            notification: "<task-notification><summary>Fixture check-in</summary></task-notification>",
            trailing_text: "\nThis is a check-in, not a completion. The fictional task may be working quietly.",
        }),
    )]
    #[case::prefixed_envelope_followed_by_guidance(
        indoc::indoc! {"
            [SYSTEM NOTIFICATION - NOT USER INPUT]
            <task-notification><summary>Fixture check-in</summary></task-notification>
            This is a check-in, not a completion. The fictional task may be working quietly.
        "},
        Some(TaskNotificationEnvelope {
            notification: "<task-notification><summary>Fixture check-in</summary></task-notification>",
            trailing_text: "\nThis is a check-in, not a completion. The fictional task may be working quietly.",
        }),
    )]
    #[case::human_prompt_quotes_notification(
        "Please inspect this quoted notification: <task-notification>Fixture text</task-notification>",
        None
    )]
    #[case::human_prompt_starts_with_quoted_notification(
        "<task-notification><summary>Fixture text</summary></task-notification>\nPlease explain this quoted notification.",
        None
    )]
    #[case::task_notification_with_system_reminder(
        "<task-notification><summary>Fixture check-in</summary></task-notification>\n<system-reminder>Fixture reminder</system-reminder>",
        Some(TaskNotificationEnvelope {
            notification: "<task-notification><summary>Fixture check-in</summary></task-notification>",
            trailing_text: "\n<system-reminder>Fixture reminder</system-reminder>",
        }),
    )]
    #[case::missing_closing_tag("<task-notification><summary>Fixture check-in</summary>", None)]
    fn extracts_only_a_complete_task_notification_envelope(
        #[case] text: &str,
        #[case] expected: Option<TaskNotificationEnvelope<'static>>,
    ) {
        assert_eq!(task_notification_envelope(text), expected);
    }
}
