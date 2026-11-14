const PEER_MESSAGE_PREFIXES: [&str; 2] = [
    "Another Claude session sent a message:",
    "Another peer session sent a message:",
];

pub(super) fn is_peer_message(prompt: Option<&str>) -> bool {
    prompt.is_some_and(|prompt| {
        PEER_MESSAGE_PREFIXES
            .iter()
            .any(|prefix| prompt.starts_with(prefix))
    })
}

#[cfg(test)]
mod tests {
    use super::is_peer_message;
    use rstest::rstest;

    #[rstest]
    #[case::claude_peer(
        Some("Another Claude session sent a message:\nPlease review this."),
        true
    )]
    #[case::peer(
        Some("Another peer session sent a message:\nPlease review this."),
        true
    )]
    #[case::human(Some("Please review this."), false)]
    #[case::missing(None, false)]
    fn identifies_peer_prompt_prefixes(#[case] prompt: Option<&str>, #[case] expected: bool) {
        assert_eq!(is_peer_message(prompt), expected);
    }
}
