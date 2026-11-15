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
