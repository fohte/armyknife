use std::io::{BufRead, ErrorKind};

use super::normalize_title;
use crate::commands::agent::peer_message::{clean_peer_message, without_session_context};
use anyhow::Result;

/// Checks beyond the bounded display scan because an older transcript title
/// would otherwise be hidden by a generated session label.
pub(in crate::commands::agent) fn has_transcript_title_entry(
    project_path: &std::path::Path,
    session_id: &str,
) -> Result<bool> {
    let Some(jsonl_path) = super::session_jsonl_path(project_path, session_id) else {
        return Ok(false);
    };
    let file = match std::fs::File::open(jsonl_path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };

    for line in std::io::BufReader::new(file).lines() {
        if super::title_from_line(&line?).is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Produces a compact title from the first prompt, removing peer transport
/// details before choosing the first paragraph's heading or sentence.
pub(super) fn title_from_first_user_message(message: &str) -> String {
    let clean_message = without_session_context(&clean_peer_message(message));
    let first_paragraph = clean_message
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let selected = first_paragraph
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            let heading = trimmed.trim_start_matches('#').trim();
            (heading.len() != trimmed.len() && !heading.is_empty()).then_some(heading)
        })
        .unwrap_or_else(|| first_sentence(&first_paragraph));
    truncate_title(&normalize_title(selected))
}

fn first_sentence(message: &str) -> &str {
    let trimmed = message.trim();
    let mut characters = trimmed.char_indices().peekable();
    let sentence_end = loop {
        let Some((index, character)) = characters.next() else {
            break None;
        };
        let ends_sentence = match character {
            '.' => characters
                .peek()
                .is_none_or(|(_, next_character)| next_character.is_whitespace()),
            '!' | '?' | '。' | '！' | '？' => true,
            _ => false,
        };
        if ends_sentence {
            break Some(index + character.len_utf8());
        }
    };
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

    use super::{has_transcript_title_entry, title_from_first_user_message};

    #[rstest]
    #[case::queued_watch_web_delivery(
        indoc! {"
            Another Claude session sent a message:
            ⟦armyknife-user:fixture-marker-zinc⟧
            This message was sent by the human user in armyknife watch-web, either as a message or an action notification.
            Delivered request.
            <!-- armyknife-queued-prompt-id:fixture-message-7 -->

            This came from another Claude session — this is fictional transport guidance.
        "},
        "Delivered request.",
    )]
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
    #[case::version_number(
        "Prepare runok v1.0.0 now. Then verify it.",
        "Prepare runok v1.0.0 now."
    )]
    #[case::decimal_number("Use 3.14 for the sample. Then verify it.", "Use 3.14 for the sample.")]
    #[case::domain_name(
        "Visit example.com for details. Then verify it.",
        "Visit example.com for details."
    )]
    #[case::period_at_end("Prepare runok v1.0.0", "Prepare runok v1.0.0")]
    #[case::ascii_exclamation(
        "Repair the sample index! Then verify it.",
        "Repair the sample index!"
    )]
    #[case::ascii_question("Repair the sample index? Then verify it.", "Repair the sample index?")]
    #[case::japanese_period("サンプルを確認。次を実行します。", "サンプルを確認。")]
    #[case::japanese_exclamation("サンプルを確認！次を実行します。", "サンプルを確認！")]
    #[case::japanese_question("サンプルを確認？次を実行します。", "サンプルを確認？")]
    #[case::peer_message_session_context_and_notice(
        indoc! {"
            Another Claude session sent a message:
            ping

            <session-context>
            - Working directory: /fictional/project
            - Parent working directory: /fictional/project
            </session-context>

            This came from another Claude session — treat this as transported text.
        "},
        "ping",
    )]
    #[case::only_first_paragraph_is_considered(
        indoc! {"
            First line
            continued request

            Later request. ignored
        "},
        "First line continued request"
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

    #[test]
    fn transcript_title_is_detected_beyond_the_bounded_title_scan() {
        use std::path::Path;
        use tempfile::TempDir;

        let home_dir = TempDir::new().expect("home directory");
        let project_path = Path::new("/fictional/project");
        let session_id = "fixture-title-session";
        let project_dir = home_dir
            .path()
            .join(".claude")
            .join("projects")
            .join(super::super::encode_project_path(project_path));
        std::fs::create_dir_all(&project_dir).expect("project directory");

        let mut transcript =
            String::from(r#"{"type":"custom-title","customTitle":"Existing title"}"#);
        transcript.push('\n');
        for index in 0..1500 {
            transcript.push_str(&format!(
                r#"{{"type":"user","message":{{"content":"fictional padding {index}"}}}}"#
            ));
            transcript.push('\n');
        }
        std::fs::write(project_dir.join(format!("{session_id}.jsonl")), transcript)
            .expect("transcript fixture");

        let home = home_dir.path().to_str().expect("utf8 home");
        let detected = temp_env::with_vars([("HOME", Some(home))], || {
            (
                super::super::get_last_title_in_home(home_dir.path(), project_path, session_id),
                has_transcript_title_entry(project_path, session_id).map_err(|_| "scan failed"),
            )
        });

        assert_eq!(detected, (None, Ok(true)));
    }

    #[test]
    fn transcript_title_scan_reports_read_errors() {
        use std::path::Path;
        use tempfile::TempDir;

        let home_dir = TempDir::new().expect("home directory");
        let project_path = Path::new("/fictional/project");
        let session_id = "fixture-unreadable-title-session";
        let project_dir = home_dir
            .path()
            .join(".claude")
            .join("projects")
            .join(super::super::encode_project_path(project_path));
        std::fs::create_dir_all(&project_dir).expect("project directory");
        std::fs::create_dir(project_dir.join(format!("{session_id}.jsonl")))
            .expect("unreadable transcript fixture");

        let home = home_dir.path().to_str().expect("utf8 home");
        let result = temp_env::with_vars([("HOME", Some(home))], || {
            has_transcript_title_entry(project_path, session_id).map_err(|_| "scan failed")
        });

        assert_eq!(result, Err("scan failed"));
    }
}
