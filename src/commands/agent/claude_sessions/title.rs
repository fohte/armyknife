use super::normalize_title;
use crate::commands::agent::peer_message::{
    ANOTHER_CLAUDE_SESSION_MESSAGE_PREFIX, ANOTHER_PEER_SESSION_MESSAGE_PREFIX,
    PEER_MESSAGE_END_TAG, PEER_MESSAGE_START_TAG, is_peer_message,
};

/// Produces a compact title from the first prompt, removing peer transport
/// details before choosing its first heading or sentence.
pub(super) fn title_from_first_user_message(message: &str) -> String {
    let clean_message = clean_peer_message(message);
    let selected = clean_message
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            let heading = trimmed.trim_start_matches('#').trim();
            (heading.len() != trimmed.len() && !heading.is_empty()).then_some(heading)
        })
        .unwrap_or_else(|| first_sentence(&clean_message));
    truncate_title(&normalize_title(selected))
}

pub(in crate::commands::agent) fn clean_peer_message(message: &str) -> String {
    if !is_peer_message(Some(message)) {
        return message.trim().to_string();
    }

    let mut body = message.trim().to_string();
    for prefix in [
        ANOTHER_CLAUDE_SESSION_MESSAGE_PREFIX,
        ANOTHER_PEER_SESSION_MESSAGE_PREFIX.trim_end(),
    ] {
        if let Some(rest) = body.strip_prefix(prefix) {
            body = rest.trim_start_matches(['\r', '\n', ' ']).to_string();
            break;
        }
    }

    if let Some(rest) = body.strip_prefix(PEER_MESSAGE_START_TAG) {
        body = rest.to_string();
        let lines = body.lines().collect::<Vec<_>>();
        if let Some(blank_line) = lines
            .iter()
            .enumerate()
            .find_map(|(index, line)| (index > 0 && line.trim().is_empty()).then_some(index))
        {
            body = lines
                .iter()
                .skip(blank_line + 1)
                .copied()
                .collect::<Vec<_>>()
                .join("\n");
        }
        if let Some((content, _)) = body.rsplit_once(PEER_MESSAGE_END_TAG) {
            body = content.to_string();
        }
    }

    let mut lines = body.lines();
    let first = lines.next().unwrap_or_default();
    let marker_line = first.starts_with("⟦armyknife-user:") && first.ends_with('⟧');
    if marker_line {
        if let Some(content) =
            crate::commands::agent::watch_web_user_marker::strip_message(first, &body)
        {
            return content.trim().to_string();
        }
        return lines.collect::<Vec<_>>().join("\n").trim().to_string();
    }

    body.trim().to_string()
}

fn first_sentence(message: &str) -> &str {
    let trimmed = message.trim();
    let sentence_end = trimmed.char_indices().find_map(|(index, character)| {
        matches!(character, '.' | '!' | '?' | '。' | '！' | '？')
            .then_some(index + character.len_utf8())
    });
    sentence_end.map_or(trimmed, |end| &trimmed[..end])
}

fn truncate_title(title: &str) -> String {
    const MAX_TITLE_CHARS: usize = 80;
    let mut chars = title.chars();
    let truncated = chars.by_ref().take(MAX_TITLE_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!(
            "{}…",
            truncated
                .chars()
                .take(MAX_TITLE_CHARS - 1)
                .collect::<String>()
        )
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::rstest;

    use super::title_from_first_user_message;

    #[rstest]
    #[case::claude_peer_heading(
        indoc! {"
            Another Claude session sent a message:
            ⟦armyknife-user:fixture-marker-delta⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            ## Refresh the sample catalog

            Please update the sample catalog.
        "},
        "Refresh the sample catalog",
    )]
    #[case::peer_envelope_sentence(
        indoc! {"
            <peer-message>
            - From session_id: fixture-sender
            - Sender engine: sample-agent

            ⟦armyknife-user:fixture-marker-epsilon⟧
            This message was typed by the human user in armyknife watch-web.
            Repair the sample index. Keep the output readable.
            </peer-message>
        "},
        "Repair the sample index.",
    )]
    #[case::ordinary_first_sentence(
        "Repair the sample index. Then verify it.",
        "Repair the sample index."
    )]
    fn first_user_message_title_removes_peer_transport_and_chooses_short_text(
        #[case] message: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(title_from_first_user_message(message), expected);
    }

    #[test]
    fn first_user_message_title_is_limited_to_80_characters() {
        let title = title_from_first_user_message(&"x".repeat(81));

        assert_eq!(title, format!("{}…", "x".repeat(79)));
    }
}
