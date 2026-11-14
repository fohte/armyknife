use std::io::Cursor;

use chrono::{DateTime, TimeZone, Utc};
use indoc::indoc;
use rstest::rstest;

use super::super::super::super::ChatItem;
use super::super::ParserAdditions;
use super::{IncrementalAppend, IncrementalClaudeParser};
use crate::commands::agent::store::{AgentDecision, AgentTodo, WatchWebQueuedPrompt};

fn fixed_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap()
}

fn empty_additions() -> ParserAdditions<'static> {
    ParserAdditions {
        session_id: "fixture-session",
        parent: None,
        pending_tool: None,
        user_marker: None,
        decisions: &[],
        queued_prompts: &[],
        peer_out_messages: &[],
        proposals: &[],
        todos: &[],
    }
}

#[rstest::fixture]
fn saved_queued_prompt() -> WatchWebQueuedPrompt {
    WatchWebQueuedPrompt {
        id: 7,
        message_id: Some("fixture-message-cyan".to_owned()),
        content: "Delivered request.".to_owned(),
        attachments: Vec::new(),
        resolved: false,
        sent_at: fixed_time(),
    }
}

fn queued_prompt_additions<'a>(queued_prompt: &'a WatchWebQueuedPrompt) -> ParserAdditions<'a> {
    ParserAdditions {
        user_marker: Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
        queued_prompts: std::slice::from_ref(queued_prompt),
        ..empty_additions()
    }
}

const ROOT_AND_CURRENT: &str = indoc! {r#"
    {"type":"user","uuid":"fixture-root","parentUuid":null,"message":{"content":"Initial request."}}
    {"type":"assistant","uuid":"fixture-current","parentUuid":"fixture-root","message":{"content":[{"type":"text","text":"Current response."}]}}
"#};

fn incremental_parser(bytes: &[u8]) -> IncrementalClaudeParser {
    IncrementalClaudeParser::new(Cursor::new(bytes), None).unwrap()
}

fn parse_all(bytes: &[u8], additions: ParserAdditions<'_>) -> super::ParsedTranscript {
    IncrementalClaudeParser::new(Cursor::new(bytes), None)
        .unwrap()
        .render_with(fixed_time(), additions)
        .unwrap()
}

fn incremental_result(
    prefix: &[u8],
    suffix: &[u8],
) -> (IncrementalAppend, super::ParsedTranscript) {
    let mut parser = incremental_parser(prefix);
    let mut transcript = prefix.to_vec();
    transcript.extend_from_slice(suffix);
    let append = parser.append_bytes(suffix, fixed_time()).unwrap();
    let parser = match append {
        IncrementalAppend::Applied => parser,
        IncrementalAppend::RestartRequired => incremental_parser(&transcript),
    };
    (
        append,
        parser.render_with(fixed_time(), empty_additions()).unwrap(),
    )
}

const PARALLEL_TOOLS_WITH_SYSTEM_AFTER_RESULT: &str = indoc! {r#"
    {"type":"user","uuid":"root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"Run two fixture tools."}}
    {"type":"assistant","uuid":"tool-call","parentUuid":"root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Read","input":{"file_path":"/fixture/a"}},{"type":"tool_use","id":"call-b","name":"Read","input":{"file_path":"/fixture/b"}}]}}
    {"type":"user","uuid":"result-a","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"first result"}]}}
    {"type":"system","uuid":"system-step","parentUuid":"result-a","timestamp":"2026-01-01T00:00:04Z","subtype":"compact_boundary"}
"#};

const INITIAL_PENDING_SIBLING_TRANSCRIPT: &str = indoc! {r#"
    {"type":"user","uuid":"root","parentUuid":null,"message":{"content":"The original request."}}
    {"type":"assistant","uuid":"current","parentUuid":"root","message":{"content":[{"type":"text","text":"The current branch response."}]}}
    {"type":"assistant","uuid":"replacement","parentUuid":"root","message":{"content":[{"type":"text","text":"The replacement response."}]}}
"#};

#[rstest]
#[case::tool_use_then_appended_tool_result(
    indoc! {r#"
        {"type":"assistant","uuid":"tool-call","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Bash","input":{"command":"echo fixture"}}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"tool-result","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"fixture output"}]}}
    "#},
    IncrementalAppend::Applied,
)]
#[case::tool_result_after_an_appended_hook_attachment(
    indoc! {r#"
        {"type":"assistant","uuid":"tool-call","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Bash","input":{"command":"echo fixture"}}]}}
    "#},
    indoc! {r#"
        {"type":"attachment","uuid":"hook-attachment","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:02Z","attachment":{"type":"hook_success"}}
        {"type":"user","uuid":"tool-result","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"fixture output"}]}}
    "#},
    IncrementalAppend::Applied,
)]
#[case::tool_result_appended_after_a_hook_attachment_in_the_initial_transcript(
    indoc! {r#"
        {"type":"assistant","uuid":"tool-call","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Bash","input":{"command":"echo fixture"}}]}}
        {"type":"attachment","uuid":"hook-attachment","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:02Z","attachment":{"type":"hook_success"}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"tool-result","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"fixture output"}]}}
    "#},
    IncrementalAppend::Applied,
)]
#[case::parallel_tool_result_continues_after_an_attachment_preserves_its_parent(
    indoc! {r#"
        {"type":"assistant","uuid":"parallel-calls","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Read","input":{"file_path":"/fixture/a"}},{"type":"tool_use","id":"call-b","name":"Read","input":{"file_path":"/fixture/b"}}]}}
        {"type":"user","uuid":"result-a","parentUuid":"parallel-calls","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"first result"}]}}
    "#},
    indoc! {r#"
        {"type":"attachment","uuid":"hook-attachment","parentUuid":"result-a","timestamp":"2026-01-01T00:00:03Z","attachment":{"type":"hook_success"}}
        {"type":"user","uuid":"result-b","parentUuid":"parallel-calls","timestamp":"2026-01-01T00:00:04Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-b","content":"second result"}]}}
    "#},
    IncrementalAppend::Applied,
)]
#[case::partial_json_at_delta_end(
    indoc! {r#"
        {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
    "#},
    r#"{"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"unfinished"#,
    IncrementalAppend::Applied,
)]
#[case::initial_final_line_has_no_newline(
    r#"{"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}"#,
    "",
    IncrementalAppend::Applied,
)]
#[case::initial_pending_sibling_completes_on_append(
    INITIAL_PENDING_SIBLING_TRANSCRIPT.trim_end_matches('\n'),
    "\n",
    IncrementalAppend::Applied,
)]
#[case::queue_operation_crosses_append_boundary(
    indoc! {r#"
        {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
        {"type":"queue-operation","operation":"enqueue","content":"Queued
    "#}.trim_end_matches('\n'),
    " request.\"}\n",
    IncrementalAppend::Applied,
)]
#[case::ask_user_question_answer_arrives_later(
    indoc! {r#"
        {"type":"assistant","uuid":"question","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"ask-1","name":"AskUserQuestion","input":{"questions":[{"question":"Choose a fixture option","options":[{"label":"First option","description":"The first fixture choice."},{"label":"Second option","description":"The second fixture choice."}]}]}}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"answer","parentUuid":"question","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_result","tool_use_id":"ask-1","content":"Your questions have been answered: \"Choose a fixture option\"=\"Second option\""}]}}
    "#},
    IncrementalAppend::Applied,
)]
#[case::background_task_notification_completes_later(
    indoc! {r#"
        {"type":"assistant","uuid":"task-call","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"task-call-id","name":"Bash","input":{"command":"run fixture task","run_in_background":true}}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"task-started","parentUuid":"task-call","timestamp":"2026-01-01T00:00:02Z","toolUseResult":{"task_id":"fixture-task"},"message":{"content":[{"type":"tool_result","tool_use_id":"task-call-id","content":"Background task started (id: fixture-task)"}]}}
        {"type":"user","uuid":"task-finished","parentUuid":"task-started","timestamp":"2026-01-01T00:00:03Z","origin":{"kind":"task-notification"},"message":{"content":"<task-notification><task-id>fixture-task</task-id><status>completed</status><summary>Fixture task completed</summary></task-notification>"}}
    "#},
    IncrementalAppend::Applied,
)]
#[case::compaction_boundary_is_appended(
    indoc! {r#"
        {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
    "#},
    indoc! {r#"
        {"type":"system","uuid":"compact","parentUuid":null,"logicalParentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","subtype":"compact_boundary"}
    "#},
    IncrementalAppend::Applied,
)]
#[case::parallel_tool_results_share_the_same_parent(
    indoc! {r#"
        {"type":"assistant","uuid":"parallel-calls","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Read","input":{"file_path":"/fixture/a"}},{"type":"tool_use","id":"call-b","name":"Read","input":{"file_path":"/fixture/b"}}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"result-a","parentUuid":"parallel-calls","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"first result"}]}}
        {"type":"user","uuid":"result-b","parentUuid":"parallel-calls","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-b","content":"second result"}]}}
        {"type":"assistant","uuid":"parallel-answer","parentUuid":"parallel-calls","timestamp":"2026-01-01T00:00:04Z","message":{"content":[{"type":"text","text":"Parallel work is complete."}]}}
    "#},
    IncrementalAppend::Applied,
)]
#[case::user_message_branching_from_an_active_ancestor_restarts(
    indoc! {r#"
        {"type":"user","uuid":"root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"The original request."}}
        {"type":"assistant","uuid":"current","parentUuid":"root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"The current branch response."}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"replacement","parentUuid":"root","timestamp":"2026-01-01T00:00:03Z","message":{"content":"The replacement request."}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::assistant_message_branching_without_tool_results_restarts(
    indoc! {r#"
        {"type":"user","uuid":"root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"The original request."}}
        {"type":"assistant","uuid":"current","parentUuid":"root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"The current branch response."}]}}
    "#},
    indoc! {r#"
        {"type":"assistant","uuid":"replacement","parentUuid":"root","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"text","text":"The replacement response."}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::unterminated_assistant_fork_restarts_without_render_error(
    indoc! {r#"
        {"type":"user","uuid":"root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"The original request."}}
        {"type":"assistant","uuid":"current","parentUuid":"root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"The current branch response."}]}}
    "#},
    r#"{"type":"assistant","uuid":"replacement","parentUuid":"root","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"text","text":"The replacement response."}]}}"#,
    IncrementalAppend::RestartRequired,
)]
#[case::parallel_tool_result_after_a_new_assistant_requires_restart(
    indoc! {r#"
        {"type":"user","uuid":"root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"Run two fixture tools."}}
        {"type":"assistant","uuid":"parallel-calls","parentUuid":"root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Read","input":{"file_path":"/fixture/a"}},{"type":"tool_use","id":"call-b","name":"Read","input":{"file_path":"/fixture/b"}}]}}
        {"type":"user","uuid":"result-a","parentUuid":"parallel-calls","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"first result"}]}}
        {"type":"assistant","uuid":"followup","parentUuid":"result-a","timestamp":"2026-01-01T00:00:04Z","message":{"content":[{"type":"text","text":"The first tool result is ready."}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"result-b","parentUuid":"parallel-calls","timestamp":"2026-01-01T00:00:05Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-b","content":"second result"}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::assistant_after_system_invalidates_tool_result_parent(
    PARALLEL_TOOLS_WITH_SYSTEM_AFTER_RESULT,
    indoc! {r#"
        {"type":"assistant","uuid":"replacement","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:05Z","message":{"content":[{"type":"text","text":"The replacement branch is active."}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::parallel_tool_result_after_system_invalidates_tool_result_parent(
    PARALLEL_TOOLS_WITH_SYSTEM_AFTER_RESULT,
    indoc! {r#"
        {"type":"user","uuid":"result-b","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:05Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-b","content":"second result"}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::task_notification_with_non_latest_parent_restarts(
    indoc! {r#"
        {"type":"assistant","uuid":"task-call","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"task-call-id","name":"Bash","input":{"command":"run fixture task","run_in_background":true}}]}}
        {"type":"user","uuid":"task-started","parentUuid":"task-call","timestamp":"2026-01-01T00:00:02Z","toolUseResult":{"task_id":"fixture-task"},"message":{"content":[{"type":"tool_result","tool_use_id":"task-call-id","content":"Background task started (id: fixture-task)"}]}}
        {"type":"assistant","uuid":"followup","parentUuid":"task-started","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"text","text":"A later assistant turn is now the tip."}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"task-finished","parentUuid":"task-started","timestamp":"2026-01-01T00:00:04Z","origin":{"kind":"task-notification"},"message":{"content":"<task-notification><task-id>fixture-task</task-id><status>completed</status><summary>Fixture task completed</summary></task-notification>"}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::tool_result_with_missing_parent_requires_restart(
    indoc! {r#"
        {"type":"user","uuid":"root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"Run a fixture tool."}}
        {"type":"assistant","uuid":"tool-call","parentUuid":"root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Read","input":{"file_path":"/fixture/a"}}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"orphan-result","parentUuid":"missing-parent","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"orphan output"}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
fn appended_transcript_matches_a_full_parse(
    #[case] prefix: &str,
    #[case] suffix: &str,
    #[case] expected_append: IncrementalAppend,
) {
    let mut transcript = prefix.as_bytes().to_vec();
    transcript.extend_from_slice(suffix.as_bytes());
    assert_eq!(
        incremental_result(prefix.as_bytes(), suffix.as_bytes()),
        (expected_append, parse_all(&transcript, empty_additions()),),
    );
}

#[test]
fn pending_sibling_renders_only_the_active_branch() {
    let transcript = INITIAL_PENDING_SIBLING_TRANSCRIPT.trim_end_matches('\n');
    let expected: super::ParsedTranscript = (
        vec![
            ChatItem::User {
                rewind_id: Some("root".to_owned()),
                text: "The original request.".to_owned(),
                timestamp: None,
                from: None,
                queued_prompt_id: None,
            },
            ChatItem::Assistant {
                text: "The replacement response.".to_owned(),
            },
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );

    assert_eq!(
        incremental_parser(transcript.as_bytes())
            .render_with(fixed_time(), empty_additions())
            .unwrap(),
        expected,
    );
}

#[test]
fn pending_json_completed_after_a_second_append_matches_a_full_parse() {
    let prefix = indoc! {r#"
        {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
    "#};
    let first_append = r#"{"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"An answer"#;
    let second_append = r#"."}]}}"#;
    let mut transcript = prefix.as_bytes().to_vec();
    transcript.extend_from_slice(first_append.as_bytes());
    transcript.extend_from_slice(second_append.as_bytes());

    let mut parser = incremental_parser(prefix.as_bytes());
    let first_result = parser
        .append_bytes(first_append.as_bytes(), fixed_time())
        .unwrap();
    let second_result = parser
        .append_bytes(second_append.as_bytes(), fixed_time())
        .unwrap();
    let actual = parser.render_with(fixed_time(), empty_additions()).unwrap();

    assert_eq!(
        (first_result, second_result, actual),
        (
            IncrementalAppend::Applied,
            IncrementalAppend::Applied,
            parse_all(&transcript, empty_additions()),
        ),
    );
}

#[rstest]
fn appended_peer_delivery_resolves_queued_prompt_after_branch_reparse(
    saved_queued_prompt: WatchWebQueuedPrompt,
) {
    let suffix = indoc! {r#"
        {"type":"assistant","uuid":"fixture-delivery-child","parentUuid":"fixture-delivery","message":{"content":[{"type":"text","text":"A child record arrived first."}]}}
        {"type":"user","uuid":"fixture-delivery","parentUuid":"fixture-root","timestamp":"2026-01-01T00:00:03Z","isMeta":true,"origin":{"kind":"peer","from":"unknown"},"message":{"content":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nEngine-formatted delivery.\n<!-- armyknife-queued-prompt-id:fixture-message-cyan -->"}}
        {"type":"assistant","uuid":"fixture-final","parentUuid":"fixture-current","message":{"content":[{"type":"text","text":"Final response."}]}}
    "#};
    let mut transcript = ROOT_AND_CURRENT.as_bytes().to_vec();
    transcript.extend_from_slice(suffix.as_bytes());
    let mut parser = incremental_parser(ROOT_AND_CURRENT.as_bytes());
    let append = parser
        .append_bytes(suffix.as_bytes(), fixed_time())
        .unwrap();
    let parser = match append {
        IncrementalAppend::Applied => parser,
        IncrementalAppend::RestartRequired => incremental_parser(&transcript),
    };
    let actual = (
        append,
        parser
            .render_with(fixed_time(), queued_prompt_additions(&saved_queued_prompt))
            .unwrap(),
    );

    assert_eq!(
        actual,
        (
            IncrementalAppend::RestartRequired,
            (
                vec![
                    ChatItem::User {
                        rewind_id: Some("fixture-root".to_owned()),
                        text: "Initial request.".to_owned(),
                        timestamp: None,
                        from: None,
                        queued_prompt_id: None,
                    },
                    ChatItem::Assistant {
                        text: "Current response.".to_owned(),
                    },
                    ChatItem::User {
                        rewind_id: Some("fixture-delivery".to_owned()),
                        text: "Engine-formatted delivery.\n<!-- armyknife-queued-prompt-id:fixture-message-cyan -->".to_owned(),
                        timestamp: Some("2026-01-01T00:00:03+00:00".to_owned()),
                        from: None,
                        queued_prompt_id: None,
                    },
                    ChatItem::Assistant {
                        text: "Final response.".to_owned(),
                    },
                ],
                Vec::new(),
                vec![7],
                Vec::new(),
            ),
        ),
    );
}

#[rstest]
fn inactive_peer_delivery_does_not_resolve_queued_prompt(
    saved_queued_prompt: WatchWebQueuedPrompt,
) {
    let suffix = indoc! {r#"
        {"type":"user","uuid":"fixture-delivery","parentUuid":"fixture-root","timestamp":"2026-01-01T00:00:03Z","isMeta":true,"origin":{"kind":"peer","from":"unknown"},"message":{"content":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nEngine-formatted delivery.\n<!-- armyknife-queued-prompt-id:fixture-message-cyan -->"}}
        {"type":"assistant","uuid":"fixture-final","parentUuid":"fixture-current","message":{"content":[{"type":"text","text":"Final response."}]}}
    "#};
    let mut transcript = ROOT_AND_CURRENT.as_bytes().to_vec();
    transcript.extend_from_slice(suffix.as_bytes());
    let mut parser = incremental_parser(ROOT_AND_CURRENT.as_bytes());
    let append = parser
        .append_bytes(suffix.as_bytes(), fixed_time())
        .unwrap();
    let parser = match append {
        IncrementalAppend::Applied => parser,
        IncrementalAppend::RestartRequired => incremental_parser(&transcript),
    };
    let actual = (
        append,
        parser
            .render_with(fixed_time(), queued_prompt_additions(&saved_queued_prompt))
            .unwrap(),
    );

    assert_eq!(
        actual,
        (
            IncrementalAppend::RestartRequired,
            (
                vec![
                    ChatItem::User {
                        rewind_id: Some("fixture-root".to_owned()),
                        text: "Initial request.".to_owned(),
                        timestamp: None,
                        from: None,
                        queued_prompt_id: None,
                    },
                    ChatItem::Assistant {
                        text: "Current response.".to_owned(),
                    },
                    ChatItem::Assistant {
                        text: "Final response.".to_owned(),
                    },
                    ChatItem::QueuedUser {
                        content: vec![super::super::super::super::UserContentPart::Text {
                            text: "Delivered request.".to_owned(),
                        }],
                        queued_prompt_id: Some("fixture-message-cyan".to_owned()),
                        from: None,
                        timestamp: Some("2026-01-01T00:00:00+00:00".to_owned()),
                    },
                ],
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        ),
    );
}

#[test]
fn overlay_changes_render_the_cached_transcript_without_reingestion() {
    let prefix = indoc! {r#"
        {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
    "#};
    let suffix = indoc! {r#"
        {"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"An answer."}]}}
    "#};
    let decision = AgentDecision {
        session_id: "fixture-session".to_owned(),
        seq: 1,
        title: "Choose a fixture option".to_owned(),
        why: "The fixture requires a choice.".to_owned(),
        considered: vec!["Option A".to_owned()],
        easy_to_change: "The fixture choice can be changed.".to_owned(),
        created_at: "2026-01-01T00:00:01.500Z".to_owned(),
        review: None,
        reviewed_at: None,
    };
    let todo = AgentTodo {
        session_id: "fixture-session".to_owned(),
        seq: 1,
        title: "Review the sample report".to_owned(),
        detail: None,
        created_at: DateTime::parse_from_rfc3339("2026-01-01T00:00:01.500Z")
            .unwrap()
            .with_timezone(&Utc),
        completed_at: None,
        resolution: None,
        comment: None,
    };
    let mut parser = incremental_parser(prefix.as_bytes());
    match parser
        .append_bytes(suffix.as_bytes(), fixed_time())
        .unwrap()
    {
        IncrementalAppend::Applied => {}
        IncrementalAppend::RestartRequired => {
            panic!("the fixture continues the active transcript branch")
        }
    }
    let additions = ParserAdditions {
        decisions: std::slice::from_ref(&decision),
        todos: std::slice::from_ref(&todo),
        ..empty_additions()
    };
    let actual = parser.render_with(fixed_time(), additions).unwrap();
    let mut transcript = prefix.as_bytes().to_vec();
    transcript.extend_from_slice(suffix.as_bytes());
    assert_eq!(actual, parse_all(&transcript, additions));
}
