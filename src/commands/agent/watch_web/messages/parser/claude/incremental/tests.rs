use std::io::Cursor;

use chrono::{DateTime, TimeZone, Utc};
use indoc::indoc;

use super::super::{
    ParseContext, ParserAdditions,
    parse_with_queued_prompts_and_peer_out_messages_and_proposals_and_skills,
};
use super::{IncrementalAppend, IncrementalClaudeParser, IncrementalClaudeSnapshot};
use crate::commands::agent::store::{AgentDecision, AgentTodo};

fn fixed_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap()
}

fn incremental_parser(bytes: &[u8]) -> IncrementalClaudeParser {
    IncrementalClaudeParser::from_reader(
        Cursor::new(bytes),
        None,
        None,
        None,
        fixed_time(),
        ParseContext {
            image_directory: None,
            queued_prompts: &[],
            peer_out_messages: &[],
            decisions: &[],
            proposals: &[],
            todos: &[],
        },
    )
    .unwrap()
}

fn full_parse(bytes: &[u8]) -> super::ParsedTranscript {
    full_parse_with(
        bytes,
        ParserAdditions {
            decisions: &[],
            queued_prompts: &[],
            peer_out_messages: &[],
            proposals: &[],
            todos: &[],
        },
    )
}

fn full_parse_with(bytes: &[u8], additions: ParserAdditions<'_>) -> super::ParsedTranscript {
    parse_with_queued_prompts_and_peer_out_messages_and_proposals_and_skills(
        Cursor::new(bytes),
        None,
        None,
        None,
        fixed_time(),
        additions,
    )
    .unwrap()
}

fn parsed_snapshot(parser: &IncrementalClaudeParser) -> super::ParsedTranscript {
    match parser.snapshot(fixed_time()).unwrap() {
        IncrementalClaudeSnapshot::Parsed(parsed) => parsed,
        IncrementalClaudeSnapshot::RestartRequired => {
            panic!("the fixture continues the active transcript branch")
        }
    }
}

#[test]
fn appended_records_match_a_full_parse() {
    let prefix = indoc! {r#"
            {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
        "#}
        .as_bytes();
    let suffix = indoc! {r#"
            {"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"An answer."}]}}
        "#}
        .as_bytes();
    let mut parser = incremental_parser(prefix);

    assert_eq!(
        parser.append_bytes(suffix, fixed_time()).unwrap(),
        IncrementalAppend::Applied,
    );
    let actual = parsed_snapshot(&parser);
    let mut transcript = prefix.to_vec();
    transcript.extend_from_slice(suffix);
    assert_eq!(actual, full_parse(&transcript));
}

#[test]
fn appended_partial_record_matches_a_full_parse() {
    let prefix = indoc! {r#"
            {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
        "#}
        .as_bytes();
    let first_part = br#"{"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"An answer."}]}}"#;
    let second_part = b"\n";
    let mut parser = incremental_parser(prefix);
    parser
        .append_bytes(first_part, fixed_time())
        .expect("the unfinished JSON line is buffered");
    assert_eq!(
        parser.append_bytes(second_part, fixed_time()).unwrap(),
        IncrementalAppend::Applied,
    );
    let actual = parsed_snapshot(&parser);
    let mut transcript = prefix.to_vec();
    transcript.extend_from_slice(first_part);
    transcript.extend_from_slice(second_part);
    assert_eq!(actual, full_parse(&transcript));
}

#[test]
fn appended_fork_requires_a_full_parse() {
    let prefix = indoc! {r#"
            {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
            {"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"An answer."}]}}
        "#}
        .as_bytes();
    let branch = indoc! {r#"
            {"type":"assistant","uuid":"message-three","parentUuid":"message-one","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"text","text":"A regenerated answer."}]}}
        "#}
        .as_bytes();
    let mut parser = incremental_parser(prefix);

    assert_eq!(
        parser.append_bytes(branch, fixed_time()).unwrap(),
        IncrementalAppend::RestartRequired,
    );
}

#[test]
fn overlay_changes_replay_the_cached_transcript() {
    let prefix = indoc! {r#"
            {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
        "#}
        .as_bytes();
    let suffix = indoc! {r#"
            {"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"An answer."}]}}
        "#}
        .as_bytes();
    let decision = AgentDecision {
        session_id: "fixture-session".to_string(),
        seq: 1,
        title: "Choose a fixture option".to_string(),
        why: "The fixture requires a choice.".to_string(),
        considered: vec!["Option A".to_string()],
        easy_to_change: "The fixture choice can be changed.".to_string(),
        created_at: "2026-01-01T00:00:01.500Z".to_string(),
        review: None,
        reviewed_at: None,
    };
    let todo = AgentTodo {
        session_id: "fixture-session".to_string(),
        seq: 1,
        title: "Review the sample report".to_string(),
        detail: None,
        created_at: DateTime::parse_from_rfc3339("2026-01-01T00:00:01.500Z")
            .unwrap()
            .with_timezone(&Utc),
        completed_at: None,
        comment: None,
    };
    let mut parser = incremental_parser(prefix);
    assert_eq!(
        parser.append_bytes(suffix, fixed_time()).unwrap(),
        IncrementalAppend::Applied,
    );
    let mut transcript = prefix.to_vec();
    transcript.extend_from_slice(suffix);
    let additions = ParserAdditions {
        decisions: std::slice::from_ref(&decision),
        queued_prompts: &[],
        peer_out_messages: &[],
        proposals: &[],
        todos: std::slice::from_ref(&todo),
    };

    let actual = match parser
        .snapshot_with(fixed_time(), None, None, None, additions)
        .unwrap()
    {
        IncrementalClaudeSnapshot::Parsed(parsed) => parsed,
        IncrementalClaudeSnapshot::RestartRequired => {
            panic!("the transcript remains on the active branch")
        }
    };
    assert_eq!(actual, full_parse_with(&transcript, additions));
}
