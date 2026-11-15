use std::collections::{BTreeSet, HashSet};

use super::cache::RunningBackgroundTaskCache;
use super::incremental::parse_session_transcript_tail_window_bounded;
use super::parser::claude::ParserAdditions;
use super::{TranscriptInput, parse_session_transcript};
use crate::commands::agent::types::{Engine, Session};

// Pending IDs restore earlier task starts, so the current turn is enough here.
const TASK_DETECTION_TAIL_MESSAGE_COUNT: usize = 1;
// Sparse transcripts must fall back without expanding the list probe.
const TASK_DETECTION_TAIL_MAX_BYTES: u64 = 4 * 1024 * 1024;

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
        let pending_task_ids = session
            .pending_bg_task_ids
            .union(&session.pending_agent_task_ids)
            .cloned()
            .collect::<BTreeSet<_>>();
        let has_running_task = cache
            .get(&session.session_id, &fingerprint, &pending_task_ids)
            .unwrap_or_else(|| {
                if fingerprint.is_none() {
                    return session.has_pending_bg_tasks();
                }

                let pending_task_ids = pending_task_ids.iter().cloned().collect::<Vec<_>>();
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
                let tail_window_result = match parse_session_transcript_tail_window_bounded(
                    &input,
                    TASK_DETECTION_TAIL_MESSAGE_COUNT,
                    &pending_task_ids,
                    TASK_DETECTION_TAIL_MAX_BYTES,
                ) {
                    Ok(Some((conversation, _))) => Some(!conversation.background_tasks.is_empty()),
                    Ok(None) => None,
                    Err(error) => {
                        tracing::warn!(
                            event = "agent.watch_web.background_task_tail_window_failed",
                            session = %session.session_id,
                            %error,
                        );
                        None
                    }
                };
                if let Some(has_running_task) = tail_window_result {
                    cache.insert(
                        &session.session_id,
                        fingerprint.clone(),
                        pending_task_ids.iter().cloned().collect(),
                        has_running_task,
                    );
                    has_running_task
                } else {
                    match parse_session_transcript(input) {
                        Ok((conversation, _)) => {
                            let has_running_task = !conversation.background_tasks.is_empty();
                            cache.insert(
                                &session.session_id,
                                fingerprint.clone(),
                                pending_task_ids.iter().cloned().collect(),
                                has_running_task,
                            );
                            has_running_task
                        }
                        Err(error) => {
                            tracing::warn!(
                                event = "agent.watch_web.background_task_transcript_parse_failed",
                                session = %session.session_id,
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
    use super::super::incremental::parse_session_transcript_tail_window_bounded;
    use super::super::parser::claude::ParserAdditions;
    use super::super::{TranscriptInput, parse_session_transcript};
    use super::running_background_task_session_ids;
    use super::{TASK_DETECTION_TAIL_MAX_BYTES, TASK_DETECTION_TAIL_MESSAGE_COUNT};
    use crate::commands::agent::types::{Engine, Session};

    #[rstest]
    #[case::background_shell("shell", false, false)]
    #[case::subagent("subagent", false, false)]
    #[case::compressed_transcript("shell", true, false)]
    #[case::completed_background_shell("shell", false, true)]
    #[case::completed_subagent("subagent", false, true)]
    fn tail_window_matches_full_parse_for_pending_tasks(
        transcript_directory: tempfile::TempDir,
        #[case] task_type: &str,
        #[case] compressed: bool,
        #[case] completed_before_window: bool,
    ) {
        let task_id = "fixture-task-id";
        let transcript =
            transcript_with_old_pending_task(task_type, task_id, completed_before_window);
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
        let tail_window_was_created = parse_session_transcript_tail_window_bounded(
            &input,
            TASK_DETECTION_TAIL_MESSAGE_COUNT,
            &pending_task_ids,
            TASK_DETECTION_TAIL_MAX_BYTES,
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
                if completed_before_window {
                    HashSet::new()
                } else {
                    HashSet::from(["fixture-session".to_string()])
                },
                !completed_before_window,
            ),
        );
    }

    #[test]
    fn task_status_is_recomputed_when_pending_ids_change() {
        let transcript_directory =
            tempfile::tempdir().expect("fixture directory should be created");
        let path = transcript_directory.path().join("fixture-transcript.jsonl");
        let transcript = transcript_with_old_pending_task("shell", "fixture-task-id", false);
        std::fs::write(&path, transcript).expect("fixture transcript should be written");
        let mut session = session_for_transcript(&path, "shell", "fixture-task-id");
        let cache = RunningBackgroundTaskCache::default();
        let before_pending_ids_change =
            running_background_task_session_ids(&[session.clone()], &cache);
        session.pending_bg_task_ids.clear();

        let after_pending_ids_change = running_background_task_session_ids(&[session], &cache);

        assert_eq!(
            (before_pending_ids_change, after_pending_ids_change),
            (
                HashSet::from(["fixture-session".to_string()]),
                HashSet::new(),
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

    fn transcript_with_old_pending_task(
        task_type: &str,
        task_id: &str,
        completed_before_window: bool,
    ) -> String {
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
        if completed_before_window {
            let notification = json!({
                "uuid": "fixture-task-notification-row",
                "parentUuid": parent_uuid,
                "type": "user",
                "timestamp": "2026-01-01T00:00:02Z",
                "origin": {"kind": "task-notification"},
                "message": {"content": format!(
                    "<task-notification><task-id>{task_id}</task-id><status>completed</status></task-notification>"
                )},
            });
            append_record(&mut transcript, &notification);
            parent_uuid = "fixture-task-notification-row".to_string();
        }
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
