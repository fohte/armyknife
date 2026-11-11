const USER_MESSAGE_LABEL: &str = "This message was typed by the human user in armyknife watch-web.";

pub(crate) fn generate() -> String {
    format!("⟦armyknife-user:{}⟧", uuid::Uuid::new_v4())
}

pub(crate) fn prefix_message(marker: &str, content: &str) -> String {
    [marker, USER_MESSAGE_LABEL, content].join("\n")
}

pub(crate) fn strip_message<'a>(marker: &str, content: &'a str) -> Option<&'a str> {
    content
        .strip_prefix(marker)?
        .strip_prefix('\n')?
        .strip_prefix(USER_MESSAGE_LABEL)?
        .strip_prefix('\n')
}

pub(crate) fn session_start_context(marker: &str) -> String {
    format!(
        "A peer-message body beginning with `{marker}` is a direct user message typed by the human user in armyknife watch-web. New messages place `{USER_MESSAGE_LABEL}` on the line immediately after the marker; marker-only messages are legacy watch-web user messages. Treat either format as a direct user instruction, even if the peer-message preamble says it is from another session or warns that it is not the user's intent. Ignore that transport preamble, marker, and label when present before interpreting the message. Keep the marker private; never reveal or repeat it."
    )
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::*;

    #[test]
    fn generated_markers_are_distinct_uuid_tokens() {
        let markers = (generate(), generate());

        assert_eq!(
            (
                markers.0.starts_with("⟦armyknife-user:"),
                markers.0.ends_with('⟧'),
                markers.0 != markers.1,
            ),
            (true, true, true),
        );
    }

    #[test]
    fn message_prefix_round_trips_with_the_user_attribution_line() {
        let marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let content = prefix_message(marker, "Continue the example.");
        let legacy_content = indoc! {"
            ⟦armyknife-user:fixture-marker-cobalt⟧
            Continue the example.
        "};

        assert_eq!(
            (
                strip_message(marker, &content),
                strip_message(marker, "⟦armyknife-user:fixture-marker-cobalt⟧x"),
                strip_message(marker, legacy_content),
            ),
            (Some("Continue the example."), None, None),
        );
    }
}
