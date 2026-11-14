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

pub(super) fn task_notification_envelope(text: &str) -> Option<&str> {
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
    envelope
        .strip_prefix("<task-notification>")?
        .strip_suffix("</task-notification>")?;
    Some(envelope)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::is_peer_message;

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
}
