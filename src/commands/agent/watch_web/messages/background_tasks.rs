use std::collections::HashSet;

use super::cache::RunningBackgroundTaskCache;
use super::incremental::parse_session_transcript_tail_window;
use super::parser::claude::ParserAdditions;
use super::range::INITIAL_EPOCH_MESSAGE_COUNT;
use super::{TranscriptInput, parse_session_transcript};
use crate::commands::agent::types::{Engine, Session};

pub(super) fn running_background_task_session_ids(
    sessions: &[Session],
    cache: &RunningBackgroundTaskCache,
) -> HashSet<String> {
    let mut session_ids = HashSet::new();
    let decisions = [];
    let queued_prompts = [];
    let peer_out_messages = [];
    let proposals = [];

    for session in sessions
        .iter()
        .filter(|session| session.engine == Engine::Claude)
    {
        let fingerprint = match RunningBackgroundTaskCache::fingerprint(session) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                tracing::warn!(
                    event = "agent.watch_web.background_task_fingerprint_failed",
                    session_id = %session.session_id,
                    %error,
                );
                if session.has_pending_bg_tasks() {
                    session_ids.insert(session.session_id.clone());
                }
                continue;
            }
        };
        let has_running_task = cache
            .get(&session.session_id, &fingerprint)
            .unwrap_or_else(|| {
                if fingerprint.is_none() {
                    return session.has_pending_bg_tasks();
                }

                let pending_task_ids = session
                    .pending_bg_task_ids
                    .union(&session.pending_agent_task_ids)
                    .cloned()
                    .collect::<Vec<_>>();
                let input = TranscriptInput {
                    path: session.transcript_path.as_deref(),
                    engine: Engine::Claude,
                    additions: ParserAdditions {
                        session_id: &session.session_id,
                        parent: None,
                        pending_tool: None,
                        user_marker: None,
                        decisions: &decisions,
                        queued_prompts: &queued_prompts,
                        peer_out_messages: &peer_out_messages,
                        proposals: &proposals,
                        todos: &[],
                        additional_cards: &[],
                    },
                };
                let tail_window_result = match parse_session_transcript_tail_window(
                    &input,
                    INITIAL_EPOCH_MESSAGE_COUNT,
                    &pending_task_ids,
                ) {
                    Ok(Some((conversation, _))) => Some(!conversation.background_tasks.is_empty()),
                    Ok(None) => None,
                    Err(error) => {
                        tracing::warn!(
                            event = "agent.watch_web.background_task_tail_window_failed",
                            session_id = %session.session_id,
                            %error,
                        );
                        None
                    }
                };
                if let Some(has_running_task) = tail_window_result {
                    cache.insert(&session.session_id, fingerprint.clone(), has_running_task);
                    has_running_task
                } else {
                    match parse_session_transcript(input) {
                        Ok((conversation, _)) => {
                            let has_running_task = !conversation.background_tasks.is_empty();
                            cache.insert(
                                &session.session_id,
                                fingerprint.clone(),
                                has_running_task,
                            );
                            has_running_task
                        }
                        Err(error) => {
                            tracing::warn!(
                                event = "agent.watch_web.background_task_transcript_parse_failed",
                                session_id = %session.session_id,
                                %error,
                            );
                            session.has_pending_bg_tasks()
                        }
                    }
                }
            });

        if has_running_task {
            session_ids.insert(session.session_id.clone());
        }
    }

    session_ids
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::Path;

    use rstest::{fixture, rstest};
    use serde_json::{Value, json};

    use super::super::cache::RunningBackgroundTaskCache;
    use super::super::incremental::parse_session_transcript_tail_window;
    use super::super::parser::claude::ParserAdditions;
    use super::super::range::INITIAL_EPOCH_MESSAGE_COUNT;
    use super::super::{TranscriptInput, parse_session_transcript};
    use super::running_background_task_session_ids;
    use crate::commands::agent::types::{Engine, Session};

    #[rstest]
    #[case::background_shell("shell", false)]
    #[case::subagent("subagent", false)]
    #[case::compressed_transcript("shell", true)]
    fn tail_window_matches_full_parse_for_pending_tasks(
        transcript_directory: tempfile::TempDir,
        #[case] task_type: &str,
        #[case] compressed: bool,
    ) {
        let task_id = "fixture-task-id";
        let transcript = transcript_with_old_pending_task(task_type, task_id);
        let extension = if compressed { "jsonl.zst" } else { "jsonl" };
        let path = transcript_directory
            .path()
            .join(format!("fixture-transcript.{extension}"));
        let contents = if compressed {
            zstd::stream::encode_all(transcript.as_bytes(), 0)
                .expect("fixture transcript should be compressed")
        } else {
            transcript.into_bytes()
        };
        std::fs::write(&path, contents).expect("fixture transcript should be written");
        let session = session_for_transcript(&path, task_type, task_id);
        let input = transcript_input(&session);
        let pending_task_ids = session
            .pending_bg_task_ids
            .union(&session.pending_agent_task_ids)
            .cloned()
            .collect::<Vec<_>>();
        let tail_window_was_created = parse_session_transcript_tail_window(
            &input,
            INITIAL_EPOCH_MESSAGE_COUNT,
            &pending_task_ids,
        )
        .expect("tail window should parse")
        .is_some();
        let full_parse_has_task = !parse_session_transcript(input)
            .expect("full transcript should parse")
            .0
            .background_tasks
            .is_empty();

        let detected_session_ids =
            running_background_task_session_ids(&[session], &RunningBackgroundTaskCache::default());

        assert_eq!(
            (
                tail_window_was_created,
                detected_session_ids,
                full_parse_has_task
            ),
            (
                !compressed,
                HashSet::from(["fixture-session".to_string()]),
                true,
            ),
        );
    }

    #[fixture]
    fn transcript_directory() -> tempfile::TempDir {
        tempfile::tempdir().expect("fixture directory should be created")
    }

    fn session_for_transcript(path: &Path, task_type: &str, task_id: &str) -> Session {
        let pending_bg_task_ids = if task_type == "shell" {
            vec![task_id]
        } else {
            Vec::new()
        };
        let pending_agent_task_ids = if task_type == "subagent" {
            vec![task_id]
        } else {
            Vec::new()
        };
        serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "transcript_path": path,
            "engine": "claude",
            "status": "stopped",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "pending_bg_task_ids": pending_bg_task_ids,
            "pending_agent_task_ids": pending_agent_task_ids,
        }))
        .expect("session fixture should deserialize")
    }

    fn transcript_input(session: &Session) -> TranscriptInput<'_> {
        TranscriptInput {
            path: session.transcript_path.as_deref(),
            engine: Engine::Claude,
            additions: ParserAdditions {
                session_id: &session.session_id,
                parent: None,
                pending_tool: None,
                user_marker: None,
                decisions: &[],
                queued_prompts: &[],
                peer_out_messages: &[],
                proposals: &[],
                todos: &[],
                additional_cards: &[],
            },
        }
    }

    fn transcript_with_old_pending_task(task_type: &str, task_id: &str) -> String {
        let mut transcript = String::new();
        let (call, result) = if task_type == "shell" {
            (
                json!({
                    "uuid": "fixture-task-call-row",
                    "parentUuid": "fixture-task-prompt-row",
                    "type": "assistant",
                    "timestamp": "2026-01-01T00:00:00Z",
                    "message": {"content": [{
                        "type": "tool_use",
                        "id": "fixture-task-tool-use-id",
                        "name": "Bash",
                        "input": {
                            "command": "watch fixture build",
                            "description": "Watch the synthetic build",
                            "run_in_background": true,
                        },
                    }]},
                }),
                json!({
                    "uuid": "fixture-task-result-row",
                    "parentUuid": "fixture-task-call-row",
                    "type": "user",
                    "timestamp": "2026-01-01T00:00:01Z",
                    "sourceToolAssistantUUID": "fixture-task-call-row",
                    "toolUseResult": {"backgroundTaskId": task_id},
                    "message": {"content": [{
                        "type": "tool_result",
                        "tool_use_id": "fixture-task-tool-use-id",
                        "content": format!("Command running in background with ID: {task_id}."),
                    }]},
                }),
            )
        } else {
            (
                json!({
                    "uuid": "fixture-task-call-row",
                    "parentUuid": "fixture-task-prompt-row",
                    "type": "assistant",
                    "timestamp": "2026-01-01T00:00:00Z",
                    "message": {"content": [{
                        "type": "tool_use",
                        "id": "fixture-task-tool-use-id",
                        "name": "Task",
                        "input": {
                            "description": "Review the synthetic task",
                            "prompt": "Review a fictional task.",
                            "subagent_type": "FixtureExplorer",
                            "run_in_background": true,
                        },
                    }]},
                }),
                json!({
                    "uuid": "fixture-task-result-row",
                    "parentUuid": "fixture-task-call-row",
                    "type": "user",
                    "timestamp": "2026-01-01T00:00:01Z",
                    "sourceToolAssistantUUID": "fixture-task-call-row",
                    "toolUseResult": {"task_id": task_id},
                    "message": {"content": [{
                        "type": "tool_result",
                        "tool_use_id": "fixture-task-tool-use-id",
                        "content": "The task was launched.",
                    }]},
                }),
            )
        };
        let prompt = json!({
            "uuid": "fixture-task-prompt-row",
            "parentUuid": null,
            "type": "user",
            "timestamp": "2026-01-01T00:00:00Z",
            "message": {"content": "Start a synthetic background task."},
        });
        for record in [prompt, call, result] {
            append_record(&mut transcript, &record);
        }
        let mut parent_uuid = "fixture-task-result-row".to_string();
        for index in 0..100 {
            let uuid = format!("fixture-recent-message-{index}");
            let record = json!({
                "uuid": uuid,
                "parentUuid": parent_uuid,
                "type": "user",
                "timestamp": "2026-01-01T00:01:00Z",
                "message": {"content": format!("Synthetic message {index}.")},
            });
            append_record(&mut transcript, &record);
            parent_uuid = uuid;
        }
        transcript
    }

    fn append_record(transcript: &mut String, record: &Value) {
        transcript
            .push_str(&serde_json::to_string(record).expect("fixture record should serialize"));
        transcript.push('\n');
    }
}
