use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::Value;

#[cfg(test)]
use crate::commands::agent::store::{AgentDecision, WatchWebPeerOutMessage, WatchWebQueuedPrompt};
use crate::commands::agent::types::Engine;
#[cfg(test)]
use crate::commands::agent::types::Session;
#[cfg(test)]
use axum::Router;

pub(super) mod answer_images;
mod background_task_outputs;
mod background_tasks;
mod command_source;
mod conversation;
mod incremental;
mod items;
mod parser;
mod pending_permissions;
mod proposals;
mod question_preview;
mod questions;
pub(in crate::commands::agent::watch_web) mod queued_prompt;
mod range;
mod routes;
mod skill_suggestions;
mod subagent_reports;
mod transcript_path;

pub(super) use self::conversation::conversation_contains_markdown_source;
#[cfg(test)]
use self::conversation::load_conversation_with_queued_prompts;
use self::items::FileAttachmentOrigin;
pub(super) use self::items::{BackgroundTask, BackgroundTaskKind, SkillSuggestion};
use self::items::{
    ChatItem, ConversationResponse, EventKind, ParentSession, PeerDirection, ToolGroupItem,
    ToolOperation, UserContentPart,
};
#[cfg(test)]
#[cfg(test)]
pub(super) use self::pending_permissions::MAX_PERMISSION_COMMAND_BYTES;
pub(super) use self::pending_permissions::{
    permission_request_command, permission_request_command_summary, permission_request_description,
};
#[cfg(test)]
use self::questions::QuestionChoice;
use self::questions::{QuestionPrompt, question_item_from_input};
use self::routes::MessagesRouteState;
pub(super) use self::routes::{RouteLoaders, routes_with_proposals_and_peer_out_messages};

pub(in crate::commands::agent::watch_web) mod action_event;
mod cache;
mod file_attachments;
mod session_titles;
mod tagged_message;
use self::transcript_path::archived_codex_transcript_path;

pub(super) fn registered_background_tasks(session_id: &str) -> Result<Vec<BackgroundTask>> {
    background_tasks::registered_background_tasks(session_id)
}

pub(in crate::commands::agent::watch_web::messages) fn log_tail_window_fallback(
    started: Instant,
    reason: &'static str,
) {
    tracing::info!(
        reason,
        elapsed_ms = started.elapsed().as_millis(),
        "tail window unavailable; falling back to full parse"
    );
}

pub(super) fn claude_content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) != Some("tool_result"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
fn routes_with_decisions_and_queued_prompts_and_peer_out_messages(
    load_session: impl Fn(&str) -> Result<Option<Session>> + Send + Sync + 'static,
    load_user_marker: impl Fn(&str) -> Result<Option<String>> + Send + Sync + 'static,
    load_decisions: impl Fn(&str) -> Result<Vec<AgentDecision>> + Send + Sync + 'static,
    load_queued_prompts: impl Fn(&str) -> Result<Vec<WatchWebQueuedPrompt>> + Send + Sync + 'static,
    load_peer_out_messages: impl Fn(&str) -> Result<Vec<WatchWebPeerOutMessage>> + Send + Sync + 'static,
    mark_queued_prompts_resolved: impl Fn(&str, &[i64]) -> Result<()> + Send + Sync + 'static,
) -> Router {
    routes_with_proposals_and_peer_out_messages(
        load_session,
        load_user_marker,
        RouteLoaders::new(
            load_decisions,
            load_queued_prompts,
            load_peer_out_messages,
            mark_queued_prompts_resolved,
            |_| Ok(Vec::new()),
        ),
    )
}

#[cfg(test)]
fn load_messages_with(
    session_id: &str,
    load_session: impl FnMut(&str) -> Result<Option<Session>>,
) -> Result<Option<Vec<ChatItem>>> {
    let loaders = RouteLoaders::new(
        |_| Ok(Vec::new()),
        |_| Ok(Vec::new()),
        |_| Ok(Vec::new()),
        |_, _| Ok(()),
        |_| Ok(Vec::new()),
    );
    Ok(
        load_conversation_with_queued_prompts(session_id, load_session, |_| Ok(None), &loaders)?
            .map(|conversation| conversation.messages),
    )
}

pub(in crate::commands::agent::watch_web::messages) struct TranscriptInput<'a> {
    path: Option<&'a Path>,
    engine: Engine,
    additions: parser::claude::ParserAdditions<'a>,
}

trait SeekableBufRead: BufRead + Seek {}

impl<T: BufRead + Seek> SeekableBufRead for T {}

fn open_seekable_transcript(path: &Path) -> Result<Option<Box<dyn SeekableBufRead + Send>>> {
    match File::open(path) {
        Ok(file) if path.extension().and_then(|extension| extension.to_str()) == Some("zst") => {
            // The parser rewinds after collecting UUID ancestry, so compressed input must be buffered.
            let mut decoder = zstd::stream::read::Decoder::new(file)
                .with_context(|| format!("decompress transcript {}", path.display()))?;
            let mut contents = Vec::new();
            decoder
                .read_to_end(&mut contents)
                .with_context(|| format!("decompress transcript {}", path.display()))?;
            Ok(Some(Box::new(Cursor::new(contents))))
        }
        Ok(file) => Ok(Some(Box::new(BufReader::new(file)))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("open transcript {}", path.display())),
    }
}

fn open_transcript(path: &Path, allow_compressed: bool) -> Result<Option<Box<dyn BufRead + Send>>> {
    match File::open(path) {
        Ok(file) => return reader_for(file, path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("open transcript {}", path.display()));
        }
    }

    if !allow_compressed {
        return open_archived_codex_transcript(path);
    }
    let compressed_path = compressed_path(path);
    let file = match File::open(&compressed_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return open_archived_codex_transcript(path);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("open transcript {}", compressed_path.display()));
        }
    };
    let decoder = zstd::stream::read::Decoder::new(file)
        .with_context(|| format!("decompress transcript {}", compressed_path.display()))?;
    Ok(Some(Box::new(BufReader::new(decoder))))
}

fn open_archived_codex_transcript(path: &Path) -> Result<Option<Box<dyn BufRead + Send>>> {
    let codex_home = crate::shared::dirs::codex_home_dir();
    let Some(archived_path) =
        archived_codex_transcript_path(Engine::Codex, Some(path), codex_home.as_deref())?
    else {
        return Ok(None);
    };
    match File::open(&archived_path) {
        Ok(file) => reader_for(file, &archived_path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .with_context(|| format!("open archived transcript {}", archived_path.display())),
    }
}

fn reader_for(file: File, path: &Path) -> Result<Option<Box<dyn BufRead + Send>>> {
    if path.extension().and_then(|extension| extension.to_str()) == Some("zst") {
        let decoder = zstd::stream::read::Decoder::new(file)
            .with_context(|| format!("decompress transcript {}", path.display()))?;
        return Ok(Some(Box::new(BufReader::new(decoder))));
    }
    Ok(Some(Box::new(BufReader::new(file))))
}

fn compressed_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".zst");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::header::CONTENT_TYPE;
    use axum::http::{Request, Response, StatusCode};
    use futures_util::{StreamExt, stream};
    use rstest::{fixture, rstest};
    use serde_json::json;
    use tower::ServiceExt;

    use super::cache::{MessageCache, TranscriptChange};
    use super::{
        ChatItem, MAX_PERMISSION_COMMAND_BYTES, QuestionChoice, QuestionPrompt, RouteLoaders,
        load_conversation_with_queued_prompts, load_messages_with,
        routes_with_decisions_and_queued_prompts_and_peer_out_messages,
        routes_with_proposals_and_peer_out_messages,
    };
    use crate::commands::agent::store::{
        AgentDecision, AgentFile, AgentProposal, AgentTodo, PermissionDeniedOperation,
        PermissionRequestData, ProposalResolution, TodoResolution, WatchWebPeerOutMessage,
        WatchWebQueuedPrompt,
    };
    use crate::commands::agent::types::{Engine, MAIN_THREAD_AGENT_KEY, Session};
    use crate::shared::config::Config;

    fn stored_session() -> Session {
        serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "status": "ended",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .unwrap()
    }

    #[fixture]
    fn empty_route_loaders() -> RouteLoaders {
        RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
    }

    struct OlderPageFixture {
        _directory: tempfile::TempDir,
        transcript: String,
        transcript_path: PathBuf,
        compressed_path: PathBuf,
        full_router: Router,
        tail_router: Router,
        tail_cache: MessageCache,
    }

    #[fixture]
    fn older_page_fixture(#[default(400)] record_count: usize) -> OlderPageFixture {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let compressed_path = directory.path().join("full-transcript.jsonl.zst");
        let mut records = Vec::new();
        let mut parent_uuid = None;
        for index in 0..record_count {
            let uuid = format!("fixture-message-{index}");
            records.push(json!({
                "type": "user",
                "uuid": uuid,
                "parentUuid": parent_uuid,
                "timestamp": "2026-01-01T00:00:01Z",
                "message": {"content": format!("Fixture request {index}.")},
            }));
            parent_uuid = Some(uuid);
        }
        let transcript = records
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&transcript_path, &transcript).unwrap();
        std::fs::write(
            &compressed_path,
            zstd::stream::encode_all(transcript.as_bytes(), 0).unwrap(),
        )
        .unwrap();

        let make_router = |path: &Path| {
            let mut session = stored_session();
            session.transcript_path = Some(path.to_path_buf());
            let cache = MessageCache::default();
            let mut loaders = empty_route_loaders();
            loaders.message_cache = cache.clone();
            let router = routes_with_proposals_and_peer_out_messages(
                move |_| Ok(Some(session.clone())),
                |_| Ok(None),
                loaders,
            );
            (router, cache)
        };
        let (full_router, _) = make_router(&compressed_path);
        let (tail_router, tail_cache) = make_router(&transcript_path);

        OlderPageFixture {
            _directory: directory,
            transcript,
            transcript_path,
            compressed_path,
            full_router,
            tail_router,
            tail_cache,
        }
    }

    #[rstest]
    #[case::already_archived(false)]
    #[case::archived_during_loading(true)]
    fn codex_messages_are_loaded_from_archive_after_transcript_moves(
        #[case] archive_during_loading: bool,
    ) {
        let codex_home = tempfile::tempdir().unwrap();
        let file_name = "rollout-fixture-thread.jsonl";
        let recorded_path = codex_home
            .path()
            .join("sessions/2026/01/01")
            .join(file_name);
        let archived_path = codex_home.path().join("archived_sessions").join(file_name);
        std::fs::create_dir_all(archived_path.parent().unwrap()).unwrap();
        let transcript_path = if archive_during_loading {
            std::fs::create_dir_all(recorded_path.parent().unwrap()).unwrap();
            &recorded_path
        } else {
            &archived_path
        };
        std::fs::write(
            transcript_path,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Recovered conversation."}]}}"#,
        )
        .unwrap();

        let mut session = stored_session();
        session.engine = Engine::Codex;
        session.transcript_path = Some(recorded_path.clone());
        let recorded_path_for_loader = recorded_path.clone();
        let archived_path_for_loader = archived_path.clone();
        let loaders = RouteLoaders::new(
            move |_| {
                if archive_during_loading {
                    std::fs::rename(&recorded_path_for_loader, &archived_path_for_loader)?;
                }
                Ok(Vec::new())
            },
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        );
        let codex_home_path = codex_home.path().to_str().unwrap();
        let conversation = temp_env::with_vars([("CODEX_HOME", Some(codex_home_path))], || {
            load_conversation_with_queued_prompts(
                "fixture-session",
                move |id| Ok((id == "fixture-session").then(|| session.clone())),
                |_| Ok(None),
                &loaders,
            )
        })
        .unwrap()
        .unwrap();

        assert_eq!(
            conversation.messages,
            vec![ChatItem::Assistant {
                text: "Recovered conversation.".to_string(),
            }],
        );
    }

    fn background_task_output_router(transcript_path: &Path, output_path: &Path) -> Router {
        let records = [
            json!({
                "uuid": "fixture-bash-call-row",
                "parentUuid": null,
                "type": "assistant",
                "timestamp": "2026-01-01T00:00:00Z",
                "message": {"content": [{
                    "type": "tool_use",
                    "id": "fixture-bash-tool-use-id",
                    "name": "Bash",
                    "input": {"command": "watch fixture build", "run_in_background": true},
                }]},
            }),
            json!({
                "uuid": "fixture-bash-result-row",
                "parentUuid": "fixture-bash-call-row",
                "type": "user",
                "sourceToolAssistantUUID": "fixture-bash-call-row",
                "message": {"content": [{
                    "type": "tool_result",
                    "tool_use_id": "fixture-bash-tool-use-id",
                    "content": format!(
                        "Command running in background with ID: fixture-task-id. Output is being written to: {}",
                        output_path.display(),
                    ),
                }]},
            }),
        ];
        std::fs::write(
            transcript_path,
            records
                .iter()
                .map(serde_json::to_string)
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .join("\n"),
        )
        .unwrap();
        let mut session = stored_session();
        session.transcript_path = Some(transcript_path.to_path_buf());
        routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        )
    }

    #[tokio::test]
    async fn messages_route_reuses_unchanged_transcripts_and_refreshes_changed_transcripts() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("fixture.jsonl");
        std::fs::write(
            &transcript_path,
            br#"{"type":"assistant","message":{"content":[{"type":"text","text":"First response."}]}}"#,
        )
        .unwrap();

        let mut session = stored_session();
        session.transcript_path = Some(transcript_path.clone());
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );

        let first = message_response(&router).await;
        let unchanged = message_response(&router).await;
        std::fs::write(
            &transcript_path,
            br#"{"type":"assistant","message":{"content":[{"type":"text","text":"Other response."}]}}"#,
        )
        .unwrap();
        let changed = message_response(&router).await;

        assert_eq!(
            (first, unchanged, changed),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[{"type":"assistant","text":"First response."}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null})
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[{"type":"assistant","text":"First response."}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null})
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[{"type":"assistant","text":"Other response."}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null})
                ),
            ),
        );
    }

    #[tokio::test]
    async fn messages_route_refreshes_and_orders_stored_files_before_applying_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("fixture.jsonl");
        std::fs::write(
            &transcript_path,
            indoc::indoc! {r#"
                {"type":"user","origin":{"kind":"human"},"timestamp":"2026-01-01T00:00:01Z","message":{"content":"Earlier message."}}
                {"type":"assistant","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"text","text":"Assistant response."}]}}
                {"type":"user","origin":{"kind":"human"},"timestamp":"2026-01-01T00:00:05Z","message":{"content":"Later message."}}
            "#},
        )
        .unwrap();

        let session = Session {
            transcript_path: Some(transcript_path),
            ..stored_session()
        };
        let file_available = Arc::new(AtomicBool::new(false));
        let loader_file_available = Arc::clone(&file_available);
        let file_created_at = || {
            chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:04Z")
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let files = vec![
            AgentFile {
                id: "file-fixture-id".to_string(),
                position: 0,
                name: "fixture.png".to_string(),
                caption: Some("Compare these files.".to_string()),
                mime_type: Some("image/png".to_string()),
                size: 3,
                created_at: file_created_at(),
            },
            AgentFile {
                id: "file-fixture-report-id".to_string(),
                position: 1,
                name: "fixture-report.pdf".to_string(),
                caption: Some("Compare these files.".to_string()),
                mime_type: None,
                size: 7,
                created_at: file_created_at(),
            },
        ];
        let decision = AgentDecision {
            session_id: "fixture-session".to_string(),
            seq: 1,
            title: "Keep the timeline order".to_string(),
            why: "The fixture places a decision between the file and later message.".to_string(),
            considered: Vec::new(),
            easy_to_change: "Change the fixture timestamp.".to_string(),
            created_at: "2026-01-01T00:00:04.500Z".to_string(),
            review: Some("accepted".to_string()),
            reviewed_at: Some("2026-01-01T00:00:04.500Z".to_string()),
        };
        let decision_item = json!({
            "type": "decision",
            "seq": 1,
            "title": "Keep the timeline order",
            "why": "The fixture places a decision between the file and later message.",
            "considered": [],
            "easyToChange": "Change the fixture timestamp.",
            "review": "accepted",
            "createdAt": "2026-01-01T00:00:04.500Z",
        });
        let loaders = RouteLoaders::new(
            move |_| Ok(vec![decision.clone()]),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_agent_file_loader(move |_| {
            Ok(if loader_file_available.load(Ordering::Relaxed) {
                files.clone()
            } else {
                Vec::new()
            })
        });
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            loaders,
        );

        let initial = message_response(&router).await;
        file_available.store(true, Ordering::Relaxed);
        let refreshed = message_response(&router).await;
        let cached = message_response(&router).await;
        let ranged_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages?limit=3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let ranged = response_json_snapshot(ranged_response).await;

        let expected_initial = json!({
            "messages": [
                {"type": "user", "text": "Earlier message.", "timestamp": "2026-01-01T00:00:01+00:00"},
                {"type": "assistant", "text": "Assistant response."},
                decision_item.clone(),
                {"type": "user", "text": "Later message.", "timestamp": "2026-01-01T00:00:05+00:00"},
            ],
            "backgroundTasks": [],
            "start": 0,
            "total": 4,
            "decisions": [decision_item.clone()],
            "hasUnansweredProposal": false,
            "activeQuestionIndex": null,
        });
        let expected_refreshed = json!({
            "messages": [
                {"type": "user", "text": "Earlier message.", "timestamp": "2026-01-01T00:00:01+00:00"},
                {"type": "assistant", "text": "Assistant response."},
                {
                    "type": "file_attachment",
                    "files": [{
                        "name": "fixture.png",
                        "source": "/api/sessions/fixture-session/files/file-fixture-id",
                        "size": 3,
                        "isImage": true,
                        "previewType": "image",
                        "available": true,
                    }, {
                        "name": "fixture-report.pdf",
                        "source": "/api/sessions/fixture-session/files/file-fixture-report-id",
                        "size": 7,
                        "isImage": false,
                        "previewType": "download",
                        "available": true,
                    }],
                    "caption": "Compare these files.",
                    "timestamp": "2026-01-01T00:00:04+00:00",
                },
                decision_item.clone(),
                {"type": "user", "text": "Later message.", "timestamp": "2026-01-01T00:00:05+00:00"},
            ],
            "backgroundTasks": [],
            "start": 0,
            "total": 5,
            "decisions": [decision_item.clone()],
            "hasUnansweredProposal": false,
            "activeQuestionIndex": null,
        });
        let expected_ranged = json!({
            "messages": [
                {
                    "type": "file_attachment",
                    "files": [{
                        "name": "fixture.png",
                        "source": "/api/sessions/fixture-session/files/file-fixture-id",
                        "size": 3,
                        "isImage": true,
                        "previewType": "image",
                        "available": true,
                    }, {
                        "name": "fixture-report.pdf",
                        "source": "/api/sessions/fixture-session/files/file-fixture-report-id",
                        "size": 7,
                        "isImage": false,
                        "previewType": "download",
                        "available": true,
                    }],
                    "caption": "Compare these files.",
                    "timestamp": "2026-01-01T00:00:04+00:00",
                },
                decision_item.clone(),
                {"type": "user", "text": "Later message.", "timestamp": "2026-01-01T00:00:05+00:00"},
            ],
            "backgroundTasks": [],
            "start": 2,
            "total": 5,
            "decisions": [decision_item],
            "hasUnansweredProposal": false,
            "activeQuestionIndex": null,
        });

        assert_eq!(
            (initial, refreshed, cached, ranged),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    expected_initial
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    expected_refreshed.clone(),
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    expected_refreshed,
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    expected_ranged,
                ),
            ),
        );
    }

    #[rstest]
    #[case::classifier_verdict(
        Some("[Sensitive Operation]"),
        true,
        Some("[Sensitive Operation]"),
        false
    )]
    #[case::missing_verdict(
        Some("Auto mode could not evaluate this action and is blocking it for safety."),
        false,
        Some("Auto mode could not evaluate this action and is blocking it for safety."),
        false
    )]
    #[case::oversized_command(None, false, None, true)]
    #[tokio::test]
    async fn messages_route_shows_permission_denial_and_retryability(
        #[case] reason: Option<&str>,
        #[case] retryable: bool,
        #[case] expected_reason: Option<&str>,
        #[case] oversized_command: bool,
    ) {
        let command = if oversized_command {
            format!("echo {}", "x".repeat(MAX_PERMISSION_COMMAND_BYTES))
        } else {
            indoc::indoc! {r#"
                printf 'first line'
                printf 'second line'
            "#}
            .to_string()
        };
        let expected_input_summary = command.chars().take(200).collect::<String>();
        let operation = PermissionDeniedOperation {
            session_id: "fixture-session".to_string(),
            seq: 1,
            tool_use_id: Some("tool-call-fixture".to_string()),
            tool_name: "Bash".to_string(),
            tool_input: json!({ "command": command }),
            reason: reason.map(str::to_string),
            created_at: chrono::Utc::now(),
            resolved_at: None,
            resolution: None,
        };
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_permission_denied_loader(move |_| Ok(vec![operation.clone()]));
        let router = routes_with_proposals_and_peer_out_messages(
            |id| Ok((id == "fixture-session").then(stored_session)),
            |_| Ok(None),
            loaders,
        );
        let response = message_response(&router).await;
        let mut expected_message = json!({
            "type": "permission_denied",
            "seq": 1,
            "toolUseId": "tool-call-fixture",
            "tool": "Bash",
            "inputSummary": expected_input_summary,
            "reason": expected_reason,
            "retryable": retryable,
            "resolved": false,
            "resolution": null,
        });
        if !oversized_command {
            expected_message["command"] = json!(command);
        }

        assert_eq!(
            response,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                json!({
                    "messages": [expected_message],
                    "backgroundTasks": [],
                    "start": 0,
                    "total": 1,
                    "decisions": [],
                    "hasUnansweredProposal": false,
                    "activeQuestionIndex": null,
                }),
            ),
        );
    }

    #[tokio::test]
    async fn messages_route_places_unclassified_denials_in_time_and_pins_pending_denials() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("fixture-session.jsonl");
        std::fs::write(
            &transcript_path,
            indoc::indoc! {r#"
                {"type":"assistant","timestamp":"2026-06-01T12:00:00Z","message":{"content":[{"type":"text","text":"Before the denials."}]}}
                {"type":"assistant","timestamp":"2026-06-01T12:03:00Z","message":{"content":[{"type":"text","text":"After the denials."}]}}
            "#},
        )
        .unwrap();
        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-06-01T12:01:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let unclassified_created_at = created_at + chrono::Duration::minutes(1);
        let permission_denials = vec![
            PermissionDeniedOperation {
                session_id: "fixture-session".to_string(),
                seq: 1,
                tool_use_id: Some("fixture-resolved-denial".to_string()),
                tool_name: "Bash".to_string(),
                tool_input: json!({"command": "echo fixture"}),
                reason: Some("Fixture denial".to_string()),
                created_at,
                resolved_at: Some(created_at),
                resolution: None,
            },
            PermissionDeniedOperation {
                session_id: "fixture-session".to_string(),
                seq: 3,
                tool_use_id: Some("fixture-unclassified-denial".to_string()),
                tool_name: "Bash".to_string(),
                tool_input: json!({"command": "echo fixture"}),
                reason: Some("Classifier unavailable".to_string()),
                created_at: unclassified_created_at,
                resolved_at: None,
                resolution: None,
            },
            PermissionDeniedOperation {
                session_id: "fixture-session".to_string(),
                seq: 4,
                tool_use_id: Some("fixture-dismissed-denial".to_string()),
                tool_name: "Bash".to_string(),
                tool_input: json!({"command": "echo fixture"}),
                reason: Some("Fixture denial".to_string()),
                created_at: created_at + chrono::Duration::seconds(2),
                resolved_at: Some(created_at + chrono::Duration::seconds(3)),
                resolution: Some(
                    crate::commands::agent::store::PermissionDeniedResolution::Dismissed,
                ),
            },
            PermissionDeniedOperation {
                session_id: "fixture-session".to_string(),
                seq: 2,
                tool_use_id: Some("fixture-pending-denial".to_string()),
                tool_name: "Bash".to_string(),
                tool_input: json!({"command": "echo fixture"}),
                reason: Some("Fixture denial".to_string()),
                created_at: created_at + chrono::Duration::seconds(1),
                resolved_at: None,
                resolution: None,
            },
        ];
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_permission_denied_loader(move |_| Ok(permission_denials.clone()));
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            loaders,
        );

        assert_eq!(
            message_response(&router).await,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                json!({
                    "messages": [
                        {"type": "assistant", "text": "Before the denials."},
                        {
                            "type": "permission_denied",
                            "seq": 1,
                            "toolUseId": "fixture-resolved-denial",
                            "tool": "Bash",
                            "inputSummary": "echo fixture",
                            "command": "echo fixture",
                            "reason": "Fixture denial",
                            "retryable": true,
                            "resolved": true,
                            "resolution": "approved",
                        },
                        {
                            "type": "permission_denied",
                            "seq": 4,
                            "toolUseId": "fixture-dismissed-denial",
                            "tool": "Bash",
                            "inputSummary": "echo fixture",
                            "command": "echo fixture",
                            "reason": "Fixture denial",
                            "retryable": true,
                            "resolved": true,
                            "resolution": "dismissed",
                        },
                        {
                            "type": "permission_denied",
                            "seq": 3,
                            "toolUseId": "fixture-unclassified-denial",
                            "tool": "Bash",
                            "inputSummary": "echo fixture",
                            "command": "echo fixture",
                            "reason": "Classifier unavailable",
                            "retryable": false,
                            "resolved": false,
                            "resolution": null,
                        },
                        {"type": "assistant", "text": "After the denials."},
                        {
                            "type": "permission_denied",
                            "seq": 2,
                            "toolUseId": "fixture-pending-denial",
                            "tool": "Bash",
                            "inputSummary": "echo fixture",
                            "command": "echo fixture",
                            "reason": "Fixture denial",
                            "retryable": true,
                            "resolved": false,
                            "resolution": null,
                        },
                    ],
                    "backgroundTasks": [],
                    "start": 0,
                    "total": 6,
                    "decisions": [],
                    "hasUnansweredProposal": false,
                    "activeQuestionIndex": null,
                }),
            ),
        );
    }

    #[tokio::test]
    async fn messages_route_orders_subagent_permissions_by_tool_start_time_and_refreshes_cache() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("fixture-session.jsonl");
        std::fs::write(&transcript_path, "").unwrap();
        let subagent_directory = directory.path().join("fixture-session/subagents");
        std::fs::create_dir_all(&subagent_directory).unwrap();
        let first_agent_path = subagent_directory.join("agent-agent-fixture-a.jsonl");
        let second_agent_path = subagent_directory.join("agent-agent-fixture-b.jsonl");
        let write_subagent_transcript = |path: &Path, timestamp: &str, call_id: &str| {
            std::fs::write(
                path,
                format!(
                    "{{\"type\":\"assistant\",\"timestamp\":\"{timestamp}\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"id\":\"{call_id}\",\"name\":\"Bash\",\"input\":{{\"command\":\"run fixture\"}}}}]}}}}\n"
                ),
            )
            .unwrap();
        };
        write_subagent_transcript(
            &first_agent_path,
            "2026-06-01T12:02:00Z",
            "fixture-first-agent-call",
        );
        write_subagent_transcript(
            &second_agent_path,
            "2026-06-01T12:01:00Z",
            "fixture-second-agent-call",
        );
        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        session.pending_permission_agent_ids =
            BTreeSet::from(["agent-fixture-a".to_string(), "agent-fixture-b".to_string()]);
        session.pending_permission_request_ids = BTreeMap::from([
            (
                "agent-fixture-a".to_string(),
                "permission-request-a".to_string(),
            ),
            (
                "agent-fixture-b".to_string(),
                "permission-request-b".to_string(),
            ),
        ]);
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_permission_request_data_loader(|_, request_id| {
            Ok(Some(PermissionRequestData {
                tool_name: Some("Bash".to_string()),
                tool_input: json!({"command": "run fixture"}),
                pending: request_id.starts_with("permission-request-"),
            }))
        });
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            loaders,
        );

        let first = message_response(&router).await;
        write_subagent_transcript(
            &second_agent_path,
            "2026-06-01T12:03:00Z",
            "fixture-second-agent-call",
        );
        let refreshed = message_response(&router).await;

        let permission = |request_id: &str, agent: &str| {
            json!({
                "type": "permission",
                "requestId": request_id,
                "tool": format!("Subagent {agent}: Bash(run fixture)"),
                "description": "Waiting for permission approval.",
                "command": "run fixture",
            })
        };
        let response = |messages| {
            json!({
                "messages": messages,
                "backgroundTasks": [],
                "start": 0,
                "total": 2,
                "decisions": [],
                "hasUnansweredProposal": false,
                "activeQuestionIndex": null,
            })
        };
        assert_eq!(
            (first, refreshed),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    response([
                        permission("permission-request-b", "agent-fixture-b"),
                        permission("permission-request-a", "agent-fixture-a"),
                    ]),
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    response([
                        permission("permission-request-a", "agent-fixture-a"),
                        permission("permission-request-b", "agent-fixture-b"),
                    ]),
                ),
            ),
        );
    }

    #[rstest]
    #[case::claude(false)]
    #[case::codex(true)]
    #[tokio::test]
    async fn messages_route_parses_appended_transcript_records(#[case] use_codex_transcript: bool) {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("fixture.jsonl");
        let (first_record, appended_record) = if use_codex_transcript {
            (
                json!({
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "First response."}],
                    },
                }),
                json!({
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "Appended response."}],
                    },
                }),
            )
        } else {
            (
                json!({
                    "uuid": "fixture-first-record",
                    "parentUuid": null,
                    "type": "assistant",
                    "message": {"content": [{"type": "text", "text": "First response."}]},
                }),
                json!({
                    "uuid": "fixture-appended-record",
                    "parentUuid": "fixture-first-record",
                    "type": "assistant",
                    "message": {"content": [{"type": "text", "text": "Appended response."}]},
                }),
            )
        };
        std::fs::write(&transcript_path, format!("{}\n", first_record)).unwrap();

        let mut session = stored_session();
        session.engine = if use_codex_transcript {
            Engine::Codex
        } else {
            Engine::Claude
        };
        session.transcript_path = Some(transcript_path.clone());
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        );
        let message_cache = loaders.message_cache.clone();
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            loaders,
        );

        let first = message_response(&router).await;
        let mut transcript = std::fs::OpenOptions::new()
            .append(true)
            .open(&transcript_path)
            .unwrap();
        writeln!(transcript, "{appended_record}").unwrap();
        let appended = message_response(&router).await;

        let expected_first = (
            StatusCode::OK,
            Some("application/json".to_string()),
            json!({
                "messages": [{"type": "assistant", "text": "First response."}],
                "backgroundTasks": [],
                "start": 0,
                "total": 1,
                "decisions": [],
                "hasUnansweredProposal": false,
                "activeQuestionIndex": null,
            }),
        );
        let expected_appended = (
            StatusCode::OK,
            Some("application/json".to_string()),
            json!({
                "messages": [
                    {"type": "assistant", "text": "First response."},
                    {"type": "assistant", "text": "Appended response."},
                ],
                "backgroundTasks": [],
                "start": 0,
                "total": 2,
                "decisions": [],
                "hasUnansweredProposal": false,
                "activeQuestionIndex": null,
            }),
        );

        let slot = message_cache.transcript_parser_slot("fixture-session");
        let last_change = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|entry| entry.last_change);

        assert_eq!(
            (first, appended, last_change),
            (
                expected_first,
                expected_appended,
                Some(TranscriptChange::Appended),
            ),
        );
    }

    #[tokio::test]
    async fn messages_route_renders_new_decisions_without_reparsing_the_transcript() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("fixture.jsonl");
        std::fs::write(
            &transcript_path,
            indoc::indoc! {r#"
                {"type":"assistant","timestamp":"2026-11-10T00:00:00Z","message":{"content":[{"type":"text","text":"Fixture response."}]}}
            "#},
        )
        .unwrap();
        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let decision_enabled = Arc::new(AtomicBool::new(false));
        let decision_loader_enabled = Arc::clone(&decision_enabled);
        let loaders = RouteLoaders::new(
            move |_| {
                Ok(if decision_loader_enabled.load(Ordering::Relaxed) {
                    vec![fixture_decision(1)]
                } else {
                    Vec::new()
                })
            },
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        );
        let message_cache = loaders.message_cache.clone();
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            loaders,
        );

        let initial = message_response(&router).await;
        decision_enabled.store(true, Ordering::Relaxed);
        let updated = message_response(&router).await;
        let decision = fixture_decision_json(1);
        let slot = message_cache.transcript_parser_slot("fixture-session");
        let last_change = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|entry| entry.last_change);

        assert_eq!(
            (initial, updated, last_change),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({
                        "messages": [{"type": "assistant", "text": "Fixture response."}],
                        "backgroundTasks": [],
                        "start": 0,
                        "total": 1,
                        "decisions": [],
                        "hasUnansweredProposal": false,
                        "activeQuestionIndex": null,
                    }),
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({
                        "messages": [
                            {"type": "assistant", "text": "Fixture response."},
                            decision.clone(),
                        ],
                        "backgroundTasks": [],
                        "start": 0,
                        "total": 2,
                        "decisions": [decision],
                        "hasUnansweredProposal": false,
                        "activeQuestionIndex": null,
                    }),
                ),
                Some(TranscriptChange::Unchanged),
            ),
        );
    }

    #[rstest]
    #[case::claude(false)]
    #[case::codex(true)]
    #[tokio::test]
    async fn messages_route_returns_session_skill_suggestions(
        #[case] use_codex_transcript: bool,
        #[from(empty_route_loaders)] loaders: RouteLoaders,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("fixture.jsonl");
        let record = if use_codex_transcript {
            json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "developer",
                    "content": [{
                        "type": "input_text",
                        "text": indoc::indoc! {"
                            <skills_instructions>
                            ## Skills
                            ### Available skills
                            - design-flow: Help shape a design.
                            - debug-flow: Investigate a failure.
                            </skills_instructions>
                        "},
                    }],
                },
            })
        } else {
            json!({
                "attachment": {
                    "type": "skill_listing",
                    "content": indoc::indoc! {"
                        - design-flow: Help shape a design.
                        - debug-flow: Investigate a failure.
                    "},
                },
            })
        };
        std::fs::write(&transcript_path, record.to_string()).unwrap();
        let competing_transcript_path = directory.path().join("fixture-competing.jsonl");
        let competing_record = if use_codex_transcript {
            json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "developer",
                    "content": [{
                        "type": "input_text",
                        "text": indoc::indoc! {"
                            <skills_instructions>
                            ## Skills
                            ### Available skills
                            - competing-fixture-skill: A fixture description.
                            </skills_instructions>
                        "},
                    }],
                },
            })
        } else {
            json!({
                "attachment": {
                    "type": "skill_listing",
                    "content": "- competing-fixture-skill: A fixture description.",
                },
            })
        };
        std::fs::write(&competing_transcript_path, competing_record.to_string()).unwrap();

        let mut session = stored_session();
        let engine = if use_codex_transcript {
            Engine::Codex
        } else {
            Engine::Claude
        };
        session.engine = engine;
        session.transcript_path = Some(transcript_path);
        let mut competing_session = stored_session();
        competing_session.session_id = "fixture-competing-session".to_string();
        competing_session.engine = engine;
        competing_session.transcript_path = Some(competing_transcript_path);
        competing_session.updated_at = chrono::DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let loaders = loaders.with_session_list_loader(move || Ok(vec![competing_session.clone()]));
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            loaders,
        );

        assert_eq!(
            message_response(&router).await,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                json!({
                    "messages": [],
                    "backgroundTasks": [],
                    "skills": [
                        {"name": "design-flow", "description": "Help shape a design."},
                        {"name": "debug-flow", "description": "Investigate a failure."},
                    ],
                    "start": 0,
                    "total": 0,
                    "decisions": [],
                    "hasUnansweredProposal": false,
                    "activeQuestionIndex": null,
                }),
            ),
        );
    }

    #[rstest]
    #[case::claude(Engine::Claude)]
    #[case::codex(Engine::Codex)]
    #[tokio::test]
    async fn messages_route_falls_back_to_the_latest_same_directory_skill_listing(
        #[case] engine: Engine,
        #[from(empty_route_loaders)] loaders: RouteLoaders,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let mut session = stored_session();
        session.engine = engine;
        let local_transcript_path = directory.path().join("fixture-local.jsonl");
        let older_local_transcript_path = directory.path().join("fixture-older-local.jsonl");
        let newer_global_transcript_path = directory.path().join("fixture-global.jsonl");
        let wrong_engine_transcript_path = directory.path().join("fixture-other-engine.jsonl");
        let write_listing = |path: &Path, listing_engine: Engine, name: &str| {
            let record = match listing_engine {
                Engine::Claude => json!({
                    "attachment": {
                        "type": "skill_listing",
                        "content": format!("- {name}: A fixture description."),
                    },
                }),
                Engine::Codex => json!({
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "developer",
                        "content": [{
                            "type": "input_text",
                            "text": indoc::formatdoc! {"
                                <skills_instructions>
                                ## Skills
                                ### Available skills
                                - {name}: A fixture description.
                                </skills_instructions>
                            "},
                        }],
                    },
                }),
            };
            let mut records = vec![record];
            if listing_engine == Engine::Claude && name == "local-fixture-skill" {
                records.push(json!({
                    "attachment": {
                        "type": "skill_listing",
                        "isInitial": false,
                        "content": "- local-fixture-skill: Updated fixture description.\n- added-fixture-skill: An added fixture description.",
                    },
                }));
            }
            let transcript = records
                .iter()
                .map(serde_json::Value::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(path, transcript).unwrap();
        };
        write_listing(&local_transcript_path, engine, "local-fixture-skill");
        write_listing(
            &older_local_transcript_path,
            engine,
            "older-local-fixture-skill",
        );
        write_listing(
            &newer_global_transcript_path,
            engine,
            "global-fixture-skill",
        );
        write_listing(
            &wrong_engine_transcript_path,
            match engine {
                Engine::Claude => Engine::Codex,
                Engine::Codex => Engine::Claude,
            },
            "wrong-engine-fixture-skill",
        );

        let updated_at = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let mut local = stored_session();
        local.session_id = "fixture-local-session".to_string();
        local.engine = engine;
        local.transcript_path = Some(local_transcript_path);
        local.updated_at = updated_at("2026-01-02T00:00:00Z");

        let mut older_local = stored_session();
        older_local.session_id = "fixture-older-local-session".to_string();
        older_local.engine = engine;
        older_local.transcript_path = Some(older_local_transcript_path);
        older_local.updated_at = updated_at("2026-01-01T00:00:00Z");

        let mut global = stored_session();
        global.session_id = "fixture-global-session".to_string();
        global.cwd = PathBuf::from("/fixture/other-project");
        global.engine = engine;
        global.transcript_path = Some(newer_global_transcript_path);
        global.updated_at = updated_at("2026-01-03T00:00:00Z");

        let mut wrong_engine = stored_session();
        wrong_engine.session_id = "fixture-wrong-engine-session".to_string();
        wrong_engine.engine = match engine {
            Engine::Claude => Engine::Codex,
            Engine::Codex => Engine::Claude,
        };
        wrong_engine.transcript_path = Some(wrong_engine_transcript_path);
        wrong_engine.updated_at = updated_at("2026-01-04T00:00:00Z");

        let loaders = loaders.with_session_list_loader(move || {
            Ok(vec![
                older_local.clone(),
                local.clone(),
                global.clone(),
                wrong_engine.clone(),
            ])
        });
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            loaders,
        );

        let expected_skills = match engine {
            Engine::Claude => json!([
                {
                    "name": "local-fixture-skill",
                    "description": "Updated fixture description.",
                },
                {
                    "name": "added-fixture-skill",
                    "description": "An added fixture description.",
                },
            ]),
            Engine::Codex => json!([
                {
                    "name": "local-fixture-skill",
                    "description": "A fixture description.",
                },
            ]),
        };
        assert_eq!(
            message_response(&router).await,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                json!({
                    "messages": [],
                    "backgroundTasks": [],
                    "skills": expected_skills,
                    "start": 0,
                    "total": 0,
                    "decisions": [],
                    "hasUnansweredProposal": false,
                    "activeQuestionIndex": null,
                }),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn skill_suggestions_route_selects_by_engine_and_optional_directory(
        #[from(empty_route_loaders)] loaders: RouteLoaders,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let write_listing = |path: &Path, engine: Engine, name: &str| {
            let record = match engine {
                Engine::Claude => json!({
                    "attachment": {
                        "type": "skill_listing",
                        "content": format!("- {name}: A fixture description."),
                    },
                }),
                Engine::Codex => json!({
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "developer",
                        "content": [{
                            "type": "input_text",
                            "text": indoc::formatdoc! {"
                                <skills_instructions>
                                ## Skills
                                ### Available skills
                                - {name}: A fixture description.
                                </skills_instructions>
                            "},
                        }],
                    },
                }),
            };
            std::fs::write(path, record.to_string()).unwrap();
        };
        let local_path = directory.path().join("fixture-local.jsonl");
        let global_path = directory.path().join("fixture-global.jsonl");
        let codex_path = directory.path().join("fixture-codex.jsonl");
        write_listing(&local_path, Engine::Claude, "local-fixture-skill");
        write_listing(&global_path, Engine::Claude, "global-fixture-skill");
        write_listing(&codex_path, Engine::Codex, "codex-fixture-skill");

        let updated_at = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let mut local = stored_session();
        local.session_id = "fixture-local-session".to_string();
        local.transcript_path = Some(local_path);
        local.updated_at = updated_at("2026-01-01T00:00:00Z");

        let mut global = stored_session();
        global.session_id = "fixture-global-session".to_string();
        global.cwd = PathBuf::from("/fixture/other-project");
        global.transcript_path = Some(global_path);
        global.updated_at = updated_at("2026-01-03T00:00:00Z");

        let mut codex = stored_session();
        codex.session_id = "fixture-codex-session".to_string();
        codex.engine = Engine::Codex;
        codex.transcript_path = Some(codex_path);
        codex.updated_at = updated_at("2026-01-04T00:00:00Z");

        let loaders = loaders.with_session_list_loader(move || {
            Ok(vec![local.clone(), global.clone(), codex.clone()])
        });
        let router =
            routes_with_proposals_and_peer_out_messages(|_| Ok(None), |_| Ok(None), loaders);

        let same_directory = skill_suggestions_response_with_query(
            &router,
            "engine=claude&directory=%2Ffixture%2Fproject",
        )
        .await;
        let latest = skill_suggestions_response_with_query(&router, "engine=claude").await;
        let other_engine = skill_suggestions_response_with_query(
            &router,
            "engine=codex&directory=%2Ffixture%2Fproject",
        )
        .await;

        assert_eq!(
            (same_directory, latest, other_engine),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"skills": [{
                        "name": "local-fixture-skill",
                        "description": "A fixture description.",
                    }]}),
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"skills": [{
                        "name": "global-fixture-skill",
                        "description": "A fixture description.",
                    }]}),
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"skills": [{
                        "name": "codex-fixture-skill",
                        "description": "A fixture description.",
                    }]}),
                ),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn messages_route_uses_latest_skill_listing_when_no_same_directory_source_exists(
        #[from(empty_route_loaders)] loaders: RouteLoaders,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let session = stored_session();
        let older_transcript_path = directory.path().join("fixture-older.jsonl");
        let newer_transcript_path = directory.path().join("fixture-newer.jsonl");
        for (path, name) in [
            (&older_transcript_path, "older-fixture-skill"),
            (&newer_transcript_path, "newer-fixture-skill"),
        ] {
            std::fs::write(
                path,
                json!({
                    "attachment": {
                        "type": "skill_listing",
                        "content": format!("- {name}: A fixture description."),
                    },
                })
                .to_string(),
            )
            .unwrap();
        }

        let updated_at = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let mut older = stored_session();
        older.session_id = "fixture-older-session".to_string();
        older.cwd = PathBuf::from("/fixture/older-project");
        older.transcript_path = Some(older_transcript_path);
        older.updated_at = updated_at("2026-01-02T00:00:00Z");

        let mut newer = stored_session();
        newer.session_id = "fixture-newer-session".to_string();
        newer.cwd = PathBuf::from("/fixture/newer-project");
        newer.transcript_path = Some(newer_transcript_path);
        newer.updated_at = updated_at("2026-01-03T00:00:00Z");

        let loaders =
            loaders.with_session_list_loader(move || Ok(vec![older.clone(), newer.clone()]));
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            loaders,
        );

        assert_eq!(
            message_response(&router).await,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                json!({
                    "messages": [],
                    "backgroundTasks": [],
                    "skills": [{
                        "name": "newer-fixture-skill",
                        "description": "A fixture description.",
                    }],
                    "start": 0,
                    "total": 0,
                    "decisions": [],
                    "hasUnansweredProposal": false,
                    "activeQuestionIndex": null,
                }),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn messages_route_refreshes_fallback_skills_when_a_source_session_changes(
        #[from(empty_route_loaders)] loaders: RouteLoaders,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let older_transcript_path = directory.path().join("fixture-older.jsonl");
        let newer_transcript_path = directory.path().join("fixture-newer.jsonl");
        let write_listing = |path: &Path, name: &str| {
            std::fs::write(
                path,
                json!({
                    "attachment": {
                        "type": "skill_listing",
                        "content": format!("- {name}: A fixture description."),
                    },
                })
                .to_string(),
            )
            .unwrap();
        };
        write_listing(&older_transcript_path, "older-fixture-skill");
        write_listing(&newer_transcript_path, "newer-fixture-skill");

        let updated_at = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let mut older = stored_session();
        older.session_id = "fixture-older-session".to_string();
        older.cwd = PathBuf::from("/fixture/older-project");
        older.transcript_path = Some(older_transcript_path);
        older.updated_at = updated_at("2026-01-02T00:00:00Z");

        let mut newer = stored_session();
        newer.session_id = "fixture-newer-session".to_string();
        newer.cwd = PathBuf::from("/fixture/newer-project");
        newer.transcript_path = Some(newer_transcript_path);
        newer.updated_at = updated_at("2026-01-03T00:00:00Z");

        let source_sessions = Arc::new(Mutex::new(vec![older]));
        let loader_sessions = Arc::clone(&source_sessions);
        let loaders = loaders.with_session_list_loader(move || {
            Ok(loader_sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone())
        });
        let session = stored_session();
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            loaders,
        );

        let initial = message_response(&router).await;
        *source_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = vec![newer];
        let refreshed = message_response(&router).await;

        assert_eq!(
            (initial, refreshed),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({
                        "messages": [],
                        "backgroundTasks": [],
                        "skills": [{
                            "name": "older-fixture-skill",
                            "description": "A fixture description.",
                        }],
                        "start": 0,
                        "total": 0,
                        "decisions": [],
                        "hasUnansweredProposal": false,
                        "activeQuestionIndex": null,
                    }),
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({
                        "messages": [],
                        "backgroundTasks": [],
                        "skills": [{
                            "name": "newer-fixture-skill",
                            "description": "A fixture description.",
                        }],
                        "start": 0,
                        "total": 0,
                        "decisions": [],
                        "hasUnansweredProposal": false,
                        "activeQuestionIndex": null,
                    }),
                ),
            ),
        );
    }

    #[tokio::test]
    async fn codex_messages_route_invalidates_changed_compressed_fallback_transcripts() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("fixture.jsonl");
        let compressed_path = super::compressed_path(&transcript_path);
        let write_compressed_transcript = |text: &str| {
            let transcript = format!(
                r#"{{"type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"{text}"}}]}}}}"#,
            );
            std::fs::write(
                &compressed_path,
                zstd::stream::encode_all(transcript.as_bytes(), 0).unwrap(),
            )
            .unwrap();
        };
        write_compressed_transcript("First response.");

        let mut session = stored_session();
        session.engine = Engine::Codex;
        session.transcript_path = Some(transcript_path);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );

        let first = message_response(&router).await;
        let unchanged = message_response(&router).await;
        write_compressed_transcript("Other response.");
        let changed = message_response(&router).await;

        assert_eq!(
            (first, unchanged, changed),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[{"type":"assistant","text":"First response."}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null})
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[{"type":"assistant","text":"First response."}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null})
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[{"type":"assistant","text":"Other response."}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null})
                ),
            ),
        );
    }

    #[tokio::test]
    async fn messages_route_refreshes_resolved_proposals() {
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let proposal = Arc::new(Mutex::new(AgentProposal {
            session_id: "fixture-session".to_string(),
            seq: 1,
            body: r#"{"title":"Sample proposal","summary":null,"options":[{"label":"Option A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Option B","description":null,"pros":[],"cons":[],"recommended":false}]}"#.to_string(),
            created_at,
            answer: None,
            resolved_at: None,
            resolution: None,
            comment: None,
        }));
        let proposal_loader = Arc::clone(&proposal);
        let router = routes_with_proposals_and_peer_out_messages(
            |_| Ok(Some(stored_session())),
            |_| Ok(None),
            RouteLoaders::new(
                |_| Ok(Vec::new()),
                |_| Ok(Vec::new()),
                |_| Ok(Vec::new()),
                |_, _| Ok(()),
                move |_| Ok(vec![proposal_loader.lock().unwrap().clone()]),
            ),
        );

        let unanswered = message_response(&router).await;
        {
            let mut proposal = proposal.lock().unwrap();
            proposal.body = r#"{"title":"Updated proposal","summary":null,"options":[{"label":"Updated A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Updated B","description":null,"pros":[],"cons":[],"recommended":false}]}"#.to_string();
        }
        let edited = message_response(&router).await;
        {
            let mut proposal = proposal.lock().unwrap();
            proposal.body = r#"{"title":"Sample proposal","summary":null,"options":[{"label":"Option A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Option B","description":null,"pros":[],"cons":[],"recommended":false}]}"#.to_string();
        }
        {
            let mut proposal = proposal.lock().unwrap();
            proposal.answer = Some(r#"{"choice":0,"comment":"Keep option A."}"#.to_string());
            proposal.resolved_at = Some(created_at);
        }
        let resolved = message_response(&router).await;
        {
            let mut proposal = proposal.lock().unwrap();
            proposal.answer = None;
            proposal.resolution = Some(ProposalResolution::Withdrawn);
            proposal.comment = Some("The decision changed.".to_string());
        }
        let withdrawn = message_response(&router).await;

        assert_eq!(
            (unanswered, edited, resolved, withdrawn),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[{"type":"proposal","seq":1,"title":"Sample proposal","summary":null,"options":[{"label":"Option A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Option B","description":null,"pros":[],"cons":[],"recommended":false}],"answer":null,"resolved":false,"resolution":null,"comment":null}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":true,"activeQuestionIndex":null})
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[{"type":"proposal","seq":1,"title":"Updated proposal","summary":null,"options":[{"label":"Updated A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Updated B","description":null,"pros":[],"cons":[],"recommended":false}],"answer":null,"resolved":false,"resolution":null,"comment":null}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":true,"activeQuestionIndex":null})
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[{"type":"proposal","seq":1,"title":"Sample proposal","summary":null,"options":[{"label":"Option A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Option B","description":null,"pros":[],"cons":[],"recommended":false}],"answer":{"choice":0,"comment":"Keep option A."},"resolved":true,"resolution":null,"comment":null}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null})
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[{"type":"proposal","seq":1,"title":"Sample proposal","summary":null,"options":[{"label":"Option A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Option B","description":null,"pros":[],"cons":[],"recommended":false}],"answer":null,"resolved":true,"resolution":"withdrawn","comment":"The decision changed."}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null})
                ),
            ),
        );
    }

    #[tokio::test]
    async fn messages_route_retries_resolved_queue_deletion_after_a_failure() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("fixture.jsonl");
        std::fs::write(
            &transcript_path,
            br#"{"type":"queue-operation","operation":"popAll","timestamp":"2026-01-02T00:00:00Z"}"#,
        )
        .unwrap();
        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let queued = saved_prompt(7, "Continue the fixture build.");
        let deleted_ids = Arc::new(Mutex::new(Vec::new()));
        let delete_log = Arc::clone(&deleted_ids);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            move |_| Ok(vec![queued.clone()]),
            |_| Ok(Vec::new()),
            move |_, ids| {
                let mut attempts = delete_log.lock().unwrap();
                attempts.push(ids.to_vec());
                if attempts.len() == 1 {
                    Err(anyhow::anyhow!("temporary fixture deletion error"))
                } else {
                    Ok(())
                }
            },
        );
        let first = message_response(&router).await;
        let unchanged = message_response(&router).await;

        assert_eq!(
            (first, unchanged, deleted_ids.lock().unwrap().clone(),),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[],"backgroundTasks":[],"start":0,"total":0,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null})
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[],"backgroundTasks":[],"start":0,"total":0,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null})
                ),
                vec![vec![7], vec![7]],
            ),
        );
    }

    #[rstest]
    #[case::claude_unanswered(Engine::Claude, false)]
    #[case::claude_answered(Engine::Claude, true)]
    #[case::codex_unanswered(Engine::Codex, false)]
    #[case::codex_answered(Engine::Codex, true)]
    fn proposals_without_a_transcript_position_are_appended_to_conversation(
        #[case] engine: Engine,
        #[case] resolved: bool,
    ) {
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let mut session = stored_session();
        session.engine = engine;
        let proposal = AgentProposal {
            session_id: "fixture-session".to_string(),
            seq: 1,
            body: r#"{"title":"Sample proposal","summary":null,"options":[{"label":"Option A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Option B","description":null,"pros":[],"cons":[],"recommended":false}]}"#.to_string(),
            created_at,
            answer: resolved.then(|| r#"{"choice":0,"comment":"Keep option A."}"#.to_string()),
            resolved_at: resolved.then_some(created_at),
            resolution: None,
            comment: None,
        };

        let proposals = vec![proposal];
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            move |_| Ok(proposals.clone()),
        );
        let conversation = super::load_conversation_with_queued_prompts(
            "fixture-session",
            |id| Ok((id == "fixture-session").then_some(session.clone())),
            |_| Ok(None),
            &loaders,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            serde_json::to_value(conversation.messages).unwrap(),
            json!([{
                "type": "proposal",
                "seq": 1,
                "title": "Sample proposal",
                "summary": null,
                "options": [
                    {
                        "label": "Option A",
                        "description": null,
                        "pros": [],
                        "cons": [],
                        "recommended": false,
                    },
                    {
                        "label": "Option B",
                        "description": null,
                        "pros": [],
                        "cons": [],
                        "recommended": false,
                    },
                ],
                "answer": if resolved {
                    json!({"choice": 0, "comment": "Keep option A."})
                } else {
                    serde_json::Value::Null
                },
                "resolved": resolved,
                "resolution": null,
                "comment": null,
            }]),
        );
    }

    #[tokio::test]
    async fn messages_route_includes_recorded_todos_and_completion_state() {
        let epoch = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let session = stored_session();
        let todos = Arc::new(Mutex::new(vec![
            AgentTodo {
                session_id: "fixture-session".to_string(),
                seq: 1,
                title: "Check the page".to_string(),
                detail: Some("Use a phone".to_string()),
                folded: false,
                added_folded: false,
                created_at: epoch,
                completed_at: None,
                resolution: None,
                comment: None,
            },
            AgentTodo {
                session_id: "fixture-session".to_string(),
                seq: 2,
                title: "Update settings".to_string(),
                detail: None,
                folded: false,
                added_folded: false,
                created_at: epoch,
                completed_at: Some(epoch),
                resolution: Some(TodoResolution::Completed),
                comment: Some("Done".to_string()),
            },
        ]));
        let todo_loader = Arc::clone(&todos);
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_todo_loader(move |_| Ok(todo_loader.lock().unwrap().clone()));
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            loaders,
        );

        let original = message_response(&router).await;
        {
            let mut todos = todos.lock().unwrap();
            todos[0].title = "Review the page".to_string();
            todos[0].detail = Some("Check the navigation".to_string());
        }
        let edited = message_response(&router).await;

        assert_eq!(
            (original, edited),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({
                        "messages": [
                            {
                                "type": "todo",
                                "seq": 2,
                                "title": "Update settings",
                                "detail": null,
                                "completed": true,
                                "resolution": "completed",
                                "comment": "Done",
                            },
                            {
                                "type": "todo",
                                "seq": 1,
                                "title": "Check the page",
                                "detail": "Use a phone",
                                "completed": false,
                                "resolution": null,
                                "comment": null,
                            },
                        ],
                        "backgroundTasks": [],
                        "start": 0,
                        "total": 2,
                        "decisions": [],
                        "hasUnansweredProposal": false,
                        "activeQuestionIndex": null,
                    }),
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({
                        "messages": [
                            {
                                "type": "todo",
                                "seq": 2,
                                "title": "Update settings",
                                "detail": null,
                                "completed": true,
                                "resolution": "completed",
                                "comment": "Done",
                            },
                            {
                                "type": "todo",
                                "seq": 1,
                                "title": "Review the page",
                                "detail": "Check the navigation",
                                "completed": false,
                                "resolution": null,
                                "comment": null,
                            },
                        ],
                        "backgroundTasks": [],
                        "start": 0,
                        "total": 2,
                        "decisions": [],
                        "hasUnansweredProposal": false,
                        "activeQuestionIndex": null,
                    }),
                ),
            ),
        );
    }

    fn question_choice(label: &str) -> QuestionChoice {
        QuestionChoice {
            label: label.to_string(),
            description: None,
        }
    }

    fn pending_question_transcript() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let record = json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "fixture-question-id",
                    "name": "AskUserQuestion",
                    "input": {
                        "questions": [{
                            "question": "Should the summary be brief?",
                            "header": "Summary",
                            "multiSelect": true,
                            "options": [
                                {"label": "Yes", "description": "Keep it concise."},
                                {"label": "No"},
                            ],
                        }],
                    },
                }],
            },
        });
        std::fs::write(&transcript_path, serde_json::to_vec(&record).unwrap()).unwrap();
        (directory, transcript_path)
    }

    async fn response_snapshot(response: Response<Body>) -> (StatusCode, Option<String>, String) {
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|value| value.to_str().unwrap().to_owned());
        let body = String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        (status, content_type, body)
    }

    async fn message_response(
        router: &axum::Router,
    ) -> (StatusCode, Option<String>, serde_json::Value) {
        message_response_with_query(router, "").await
    }

    async fn message_response_with_query(
        router: &axum::Router,
        query: &str,
    ) -> (StatusCode, Option<String>, serde_json::Value) {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/sessions/fixture-session/messages{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        response_json_snapshot(response).await
    }

    async fn skill_suggestions_response_with_query(
        router: &axum::Router,
        query: &str,
    ) -> (StatusCode, Option<String>, serde_json::Value) {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/skill-suggestions?{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        response_json_snapshot(response).await
    }

    async fn response_json_snapshot(
        response: Response<Body>,
    ) -> (StatusCode, Option<String>, serde_json::Value) {
        let (status, content_type, body) = response_snapshot(response).await;
        (status, content_type, serde_json::from_str(&body).unwrap())
    }

    fn fixture_decision(seq: i64) -> AgentDecision {
        AgentDecision {
            session_id: "fixture-session".to_string(),
            seq,
            title: format!("Fixture decision {seq}"),
            why: "The fixture keeps the example isolated.".to_string(),
            considered: Vec::new(),
            easy_to_change: "Change the fixture value.".to_string(),
            created_at: "2026-11-11T00:00:00Z".to_string(),
            review: None,
            reviewed_at: None,
        }
    }

    fn fixture_decision_json(seq: i64) -> serde_json::Value {
        json!({
            "type": "decision",
            "seq": seq,
            "title": format!("Fixture decision {seq}"),
            "why": "The fixture keeps the example isolated.",
            "considered": [],
            "easyToChange": "Change the fixture value.",
            "review": null,
            "createdAt": "2026-11-11T00:00:00Z",
        })
    }

    fn ranged_response(start: usize, message_seqs: &[i64]) -> serde_json::Value {
        let messages = message_seqs
            .iter()
            .map(|seq| fixture_decision_json(*seq))
            .collect::<Vec<_>>();
        json!({
            "messages": messages,
            "backgroundTasks": [],
            "start": start,
            "total": 4,
            "decisions": (1..=4).map(fixture_decision_json).collect::<Vec<_>>(),
            "hasUnansweredProposal": false,
            "activeQuestionIndex": null,
        })
    }

    #[tokio::test]
    async fn messages_route_returns_requested_ranges_with_full_conversation_metadata() {
        let session = Session {
            engine: Engine::Codex,
            ..stored_session()
        };
        let decisions = (1..=4).map(fixture_decision).collect::<Vec<_>>();
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            move |_| Ok(decisions.clone()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );
        let mut responses = Vec::new();
        for query in ["", "?limit=2", "?before=2&limit=2", "?from=2"] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/sessions/fixture-session/messages{query}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            responses.push(response_json_snapshot(response).await);
        }

        assert_eq!(
            responses,
            vec![
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    ranged_response(0, &[1, 2, 3, 4]),
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    ranged_response(2, &[3, 4]),
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    ranged_response(0, &[1, 2]),
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    ranged_response(2, &[3, 4]),
                ),
            ],
        );
    }

    #[tokio::test]
    async fn messages_route_reuses_stable_range_digest_and_falls_back_when_changed() {
        let session = Session {
            engine: Engine::Codex,
            ..stored_session()
        };
        let original_decisions = (1..=22).map(fixture_decision).collect::<Vec<_>>();
        let mut changed_decisions = original_decisions.clone();
        changed_decisions[0].title = "Updated fixture decision".to_string();
        let request_count = Arc::new(AtomicUsize::new(0));
        let request_count_for_loader = Arc::clone(&request_count);
        let decisions_for_loader = [
            original_decisions.clone(),
            original_decisions.clone(),
            changed_decisions.clone(),
        ];
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            move |_| {
                let index = request_count_for_loader.fetch_add(1, Ordering::SeqCst);
                Ok(decisions_for_loader[index.min(2)].clone())
            },
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );
        let first_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages?limit=100")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let mut responses = vec![response_json_snapshot(first_response).await];
        let (stable_start, stable_end, stable_digest) = {
            let first_body = &responses[0].2;
            (
                first_body["stableStart"].as_u64().unwrap(),
                first_body["stableEnd"].as_u64().unwrap(),
                first_body["stableDigest"].as_str().unwrap().to_string(),
            )
        };
        for _ in 0..2 {
            let query = format!(
                "?from=0&stableStart={stable_start}&stableEnd={stable_end}&stableDigest={stable_digest}"
            );
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/sessions/fixture-session/messages{query}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            responses.push(response_json_snapshot(response).await);
        }

        let original_json = (1..=22).map(fixture_decision_json).collect::<Vec<_>>();
        let mut changed_json = original_json.clone();
        changed_json[0]["title"] = json!("Updated fixture decision");
        for (_, _, body) in &mut responses {
            body["stableDigest"] = json!("fixture-digest");
        }
        let response =
            |start: usize, messages: Vec<serde_json::Value>, decisions: Vec<serde_json::Value>| {
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({
                        "messages": messages,
                        "backgroundTasks": [],
                        "start": start,
                        "total": 22,
                        "decisions": decisions,
                        "hasUnansweredProposal": false,
                        "activeQuestionIndex": null,
                        "stableStart": 0,
                        "stableEnd": 2,
                        "stableDigest": "fixture-digest",
                    }),
                )
            };
        assert_eq!(
            responses,
            vec![
                response(0, original_json.clone(), original_json.clone(),),
                response(
                    2,
                    (3..=22).map(fixture_decision_json).collect::<Vec<_>>(),
                    original_json,
                ),
                response(0, changed_json.clone(), changed_json),
            ],
        );
    }

    #[rstest]
    #[case::before_requires_limit("/sessions/fixture-session/messages?before=2")]
    #[case::from_rejects_limit("/sessions/fixture-session/messages?from=2&limit=1")]
    #[case::before_and_from_are_exclusive(
        "/sessions/fixture-session/messages?before=2&from=1&limit=1"
    )]
    #[case::stable_start_must_match_from(
        "/sessions/fixture-session/messages?from=0&stableStart=1&stableEnd=2&stableDigest=fixture"
    )]
    #[case::stable_end_must_not_precede_start(
        "/sessions/fixture-session/messages?from=1&stableStart=1&stableEnd=0&stableDigest=fixture"
    )]
    #[tokio::test]
    async fn messages_route_rejects_incompatible_range_parameters(#[case] uri: &str) {
        let session = stored_session();
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );
        let response = router
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response).await,
            (StatusCode::BAD_REQUEST, None, String::new()),
        );
    }
    #[tokio::test]
    async fn claude_message_route_places_database_decisions_in_the_transcript() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            indoc::indoc! {r#"
                {"type":"assistant","timestamp":"2026-11-11T10:00:00Z","message":{"content":[{"type":"text","text":"Before the decision."}]}}
                {"type":"assistant","timestamp":"2026-11-11T11:00:00Z","message":{"content":[{"type":"tool_use","id":"fixture-decision-call","name":"Bash","input":{"command":"a agent decision add 'Choose the fixture path'"}}]}}
                {"type":"user","timestamp":"2026-11-11T12:00:00Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-decision-call","content":"Recorded decision 5"}]}}
                {"type":"assistant","timestamp":"2026-11-11T13:00:00Z","message":{"content":[{"type":"text","text":"After the decision."}]}}
            "#},
        )
        .unwrap();
        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            |_| {
                Ok(vec![AgentDecision {
                    session_id: "fixture-session".to_string(),
                    seq: 5,
                    title: "Choose the fixture path".to_string(),
                    why: "The fixture remains reversible.".to_string(),
                    considered: vec!["Choose the other fixture path".to_string()],
                    easy_to_change: "Change one fixture value.".to_string(),
                    created_at: "2026-11-11T12:30:00Z".to_string(),
                    review: Some("ok".to_string()),
                    reviewed_at: None,
                }])
            },
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response).await,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                r#"{"messages":[{"type":"assistant","text":"Before the decision."},{"type":"tool_summary","summary":"Bash ×1","operations":[{"tool":"Bash","target":"a agent decision add 'Choose the fixture path'","failed":false}]},{"type":"decision","seq":5,"title":"Choose the fixture path","why":"The fixture remains reversible.","considered":["Choose the other fixture path"],"easyToChange":"Change one fixture value.","review":"ok","createdAt":"2026-11-11T12:30:00Z"},{"type":"assistant","text":"After the decision."}],"backgroundTasks":[],"start":0,"total":4,"decisions":[{"type":"decision","seq":5,"title":"Choose the fixture path","why":"The fixture remains reversible.","considered":["Choose the other fixture path"],"easyToChange":"Change one fixture value.","review":"ok","createdAt":"2026-11-11T12:30:00Z"}],"hasUnansweredProposal":false,"activeQuestionIndex":null}"#.to_string(),
            ),
        );
    }

    #[tokio::test]
    async fn claude_message_route_places_decisions_and_proposals_by_creation_time() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            indoc::indoc! {r#"
                {"type":"assistant","timestamp":"2026-06-01T11:58:00Z","message":{"content":[{"type":"text","text":"Before the records."}]}}
                {"type":"assistant","timestamp":"2026-06-01T11:59:30Z","message":{"content":[{"type":"tool_use","id":"fixture-decision-call","name":"Bash","input":{"command":"a agent decision add 'Choose the fixture path'"}}]}}
                {"type":"user","timestamp":"2026-06-01T12:00:00Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-decision-call","content":"Recorded decision 5"}]}}
                {"type":"assistant","timestamp":"2026-06-01T12:00:30Z","message":{"content":[{"type":"text","text":"Between the records."}]}}
                {"type":"assistant","timestamp":"2026-06-01T12:01:00Z","message":{"content":[{"type":"tool_use","id":"fixture-proposal-call","name":"Bash","input":{"command":"a agent proposal add '<fixture-json>'"}}]}}
                {"type":"user","timestamp":"2026-06-01T12:02:00Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-proposal-call","content":"Recorded proposal 7"}]}}
                {"type":"assistant","timestamp":"2026-06-01T12:03:00Z","message":{"content":[{"type":"text","text":"After the records."}]}}
            "#},
        )
        .unwrap();

        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let decision = AgentDecision {
            session_id: "fixture-session".to_string(),
            seq: 5,
            title: "Choose the fixture path".to_string(),
            why: "The fixture remains reversible.".to_string(),
            considered: vec!["Choose the other fixture path".to_string()],
            easy_to_change: "Change one fixture value.".to_string(),
            created_at: "2026-06-01T11:59:00Z".to_string(),
            review: Some("accepted".to_string()),
            reviewed_at: Some("2026-06-01T12:04:00Z".to_string()),
        };
        let proposal = AgentProposal {
            session_id: "fixture-session".to_string(),
            seq: 7,
            body: r#"{"title":"Choose a fixture option","summary":null,"options":[{"label":"Option A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Option B","description":null,"pros":[],"cons":[],"recommended":false}]}"#.to_string(),
            created_at: chrono::DateTime::parse_from_rfc3339("2026-06-01T12:00:45Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            answer: None,
            resolved_at: None,
            resolution: None,
            comment: None,
        };
        let router = routes_with_proposals_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            RouteLoaders::new(
                move |_| Ok(vec![decision.clone()]),
                |_| Ok(Vec::new()),
                |_| Ok(Vec::new()),
                |_, _| Ok(()),
                move |_| Ok(vec![proposal.clone()]),
            ),
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, content_type, body) = response_snapshot(response).await;

        assert_eq!(
            (
                status,
                content_type,
                serde_json::from_str::<serde_json::Value>(&body).unwrap(),
            ),
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                json!({
                    "messages": [
                        {"type": "assistant", "text": "Before the records."},
                        {
                            "type": "decision",
                            "seq": 5,
                            "title": "Choose the fixture path",
                            "why": "The fixture remains reversible.",
                            "considered": ["Choose the other fixture path"],
                            "easyToChange": "Change one fixture value.",
                            "review": "accepted",
                            "createdAt": "2026-06-01T11:59:00Z",
                        },
                        {
                            "type": "tool_summary",
                            "summary": "Bash ×1",
                            "operations": [{
                                "tool": "Bash",
                                "target": "a agent decision add 'Choose the fixture path'",
                                "failed": false,
                            }],
                        },
                        {"type": "assistant", "text": "Between the records."},
                        {
                            "type": "tool_summary",
                            "summary": "Bash ×1",
                            "operations": [{
                                "tool": "Bash",
                                "target": "a agent proposal add '<fixture-json>'",
                                "failed": false,
                            }],
                        },
                        {"type": "assistant", "text": "After the records."},
                        {
                            "type": "proposal",
                            "seq": 7,
                            "title": "Choose a fixture option",
                            "summary": null,
                            "options": [
                                {
                                    "label": "Option A",
                                    "description": null,
                                    "pros": [],
                                    "cons": [],
                                    "recommended": false,
                                },
                                {
                                    "label": "Option B",
                                    "description": null,
                                    "pros": [],
                                    "cons": [],
                                    "recommended": false,
                                },
                            ],
                            "answer": null,
                            "resolved": false,
                            "resolution": null,
                            "comment": null,
                        },
                    ],
                    "backgroundTasks": [],
                    "start": 0,
                    "total": 7,
                    "decisions": [{
                        "type": "decision",
                        "seq": 5,
                        "title": "Choose the fixture path",
                        "why": "The fixture remains reversible.",
                        "considered": ["Choose the other fixture path"],
                        "easyToChange": "Change one fixture value.",
                        "review": "accepted",
                        "createdAt": "2026-06-01T11:59:00Z",
                    }],
                    "hasUnansweredProposal": true,
                    "activeQuestionIndex": null,
                }),
            ),
        );
    }

    #[tokio::test]
    async fn claude_message_route_keeps_combined_tools_and_places_database_peer_messages() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let transcript = indoc::indoc! {r#"
            {"type":"assistant","timestamp":"2026-06-01T12:00:00Z","message":{"content":[{"type":"tool_use","id":"fixture-combined-call","name":"Bash","input":{"command":"a agent decision add 'Choose the fixture path' && a agent peer notify -m \"Requested fixture body.\" fixture-target-id"}}]}}
            {"type":"user","timestamp":"2026-06-01T12:00:04Z","sourceToolAssistantUUID":"fixture-assistant-call","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-combined-call","content":"Decision recorded for this session.\nSession-local sequence: 5\nInjected into the running turn of Codex session fixture-target-id."}]}}
        "#};
        std::fs::write(&transcript_path, transcript).unwrap();

        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let sent_at = chrono::DateTime::parse_from_rfc3339("2026-06-01T12:00:03Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            |_| {
                Ok(vec![AgentDecision {
                    session_id: "fixture-session".to_string(),
                    seq: 5,
                    title: "Choose the fixture path".to_string(),
                    why: "The fixture remains reversible.".to_string(),
                    considered: vec!["Choose the other fixture path".to_string()],
                    easy_to_change: "Change one fixture value.".to_string(),
                    created_at: "2026-06-01T11:59:00Z".to_string(),
                    review: Some("accepted".to_string()),
                    reviewed_at: Some("2026-06-01T12:04:00Z".to_string()),
                }])
            },
            |_| Ok(Vec::new()),
            move |_| {
                Ok(vec![WatchWebPeerOutMessage {
                    id: 1,
                    from_session_id: "fixture-session".to_string(),
                    to_session_id: "fixture-target-id".to_string(),
                    content: "Expanded fixture body.".to_string(),
                    sent_at,
                    failed: false,
                }])
            },
            |_, _| Ok(()),
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, content_type, body) = response_snapshot(response).await;

        assert_eq!(
            (
                status,
                content_type,
                serde_json::from_str::<serde_json::Value>(&body).unwrap(),
            ),
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                json!({
                    "messages": [
                        {
                            "type": "decision",
                            "seq": 5,
                            "title": "Choose the fixture path",
                            "why": "The fixture remains reversible.",
                            "considered": ["Choose the other fixture path"],
                            "easyToChange": "Change one fixture value.",
                            "review": "accepted",
                            "createdAt": "2026-06-01T11:59:00Z",
                        },
                        {
                            "type": "peer",
                            "direction": "to",
                            "session_id": "fixture-target-id",
                            "content": [{
                                "type": "text",
                                "text": "Expanded fixture body.",
                            }],
                            "timestamp": "2026-06-01T12:00:03+00:00",
                        },
                        {
                            "type": "tool_summary",
                            "summary": "Bash ×1",
                            "operations": [{
                                "tool": "Bash",
                                "target": "a agent decision add 'Choose the fixture path' && a agent peer notify -m \"Requested fixture body.\" fixture-target-id",
                                "failed": false,
                            }],
                        },
                    ],
                    "backgroundTasks": [],
                    "start": 0,
                    "total": 3,
                    "decisions": [{
                        "type": "decision",
                        "seq": 5,
                        "title": "Choose the fixture path",
                        "why": "The fixture remains reversible.",
                        "considered": ["Choose the other fixture path"],
                        "easyToChange": "Change one fixture value.",
                        "review": "accepted",
                        "createdAt": "2026-06-01T11:59:00Z",
                    }],
                    "hasUnansweredProposal": false,
                    "activeQuestionIndex": null,
                }),
            ),
        );
    }

    #[tokio::test]
    async fn codex_message_route_places_database_decisions_by_creation_time() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            indoc::indoc! {r#"
                {"type":"response_item","timestamp":"2026-11-11T11:00:00Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Before the decision."}]}}
                {"type":"response_item","timestamp":"2026-11-11T13:00:00Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"After the decision."}]}}
            "#},
        )
        .unwrap();
        let mut session = stored_session();
        session.engine = Engine::Codex;
        session.transcript_path = Some(transcript_path);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            |_| {
                Ok(vec![AgentDecision {
                    session_id: "fixture-session".to_string(),
                    seq: 1,
                    title: "Choose the fixture path".to_string(),
                    why: "The fixture remains reversible.".to_string(),
                    considered: vec!["Choose the other fixture path".to_string()],
                    easy_to_change: "Change one fixture value.".to_string(),
                    created_at: "2026-11-11T12:00:00Z".to_string(),
                    review: Some("accepted".to_string()),
                    reviewed_at: Some("2026-11-11T12:01:00Z".to_string()),
                }])
            },
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response).await,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                r#"{"messages":[{"type":"assistant","text":"Before the decision."},{"type":"decision","seq":1,"title":"Choose the fixture path","why":"The fixture remains reversible.","considered":["Choose the other fixture path"],"easyToChange":"Change one fixture value.","review":"accepted","createdAt":"2026-11-11T12:00:00Z"},{"type":"assistant","text":"After the decision."}],"backgroundTasks":[],"start":0,"total":3,"decisions":[{"type":"decision","seq":1,"title":"Choose the fixture path","why":"The fixture remains reversible.","considered":["Choose the other fixture path"],"easyToChange":"Change one fixture value.","review":"accepted","createdAt":"2026-11-11T12:00:00Z"}],"hasUnansweredProposal":false,"activeQuestionIndex":null}"#.to_string(),
            ),
        );
    }

    fn saved_prompt(id: i64, content: &str) -> WatchWebQueuedPrompt {
        WatchWebQueuedPrompt {
            id,
            message_id: Some(format!("fixture-message-{id}")),
            content: content.to_string(),
            action_event_kind: None,
            attachments: vec![],
            resolved: false,
            sent_at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        }
    }

    #[test]
    fn missing_store_entry_has_no_messages_result() {
        assert_eq!(
            load_messages_with("fixture-unregistered-session", |_| Ok(None)).unwrap(),
            None
        );
    }

    #[rstest]
    #[case::description(
        ".jsonl",
        false,
        Some("Review the fictional feature"),
        Some("Review the fictional feature")
    )]
    #[case::compressed_description(
        ".jsonl.zst",
        true,
        Some("Review the fictional feature"),
        Some("Review the fictional feature")
    )]
    #[case::missing_metadata(".jsonl", false, None, None)]
    fn subagent_report_uses_its_description_when_metadata_is_available(
        #[case] transcript_suffix: &str,
        #[case] is_compressed: bool,
        #[case] saved_description: Option<&str>,
        #[case] expected_description: Option<&str>,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory
            .path()
            .join(format!("fixture-session{transcript_suffix}"));
        let transcript = json!({
            "type": "user",
            "isMeta": true,
            "origin": {
                "kind": "peer",
                "from": "fixture-agent-id",
                "name": "general-purpose",
                "handback": true,
                "handbackReportText": "The fictional task is complete.",
            },
            "message": {"content": "[Subagent hand-back] boilerplate"},
        });
        let transcript = serde_json::to_vec(&transcript).unwrap();
        let transcript = if is_compressed {
            zstd::stream::encode_all(transcript.as_slice(), 0).unwrap()
        } else {
            transcript
        };
        std::fs::write(&transcript_path, transcript).unwrap();
        if let Some(description) = saved_description {
            let metadata_dir = directory.path().join("fixture-session").join("subagents");
            std::fs::create_dir_all(&metadata_dir).unwrap();
            std::fs::write(
                metadata_dir.join("agent-fixture-agent-id.meta.json"),
                serde_json::to_vec(&json!({"description": description})).unwrap(),
            )
            .unwrap();
        }

        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        assert_eq!(
            load_messages_with("fixture-session", |id| {
                Ok((id == "fixture-session").then(|| session.clone()))
            })
            .unwrap(),
            Some(vec![ChatItem::SubagentReport {
                agent_id: "fixture-agent-id".to_string(),
                agent_name: "general-purpose".to_string(),
                description: expected_description.map(str::to_string),
                text: "The fictional task is complete.".to_string(),
            }])
        );
    }

    #[rstest]
    #[case::bash_command("Bash(cargo test -p sample)", "Bash", Some("cargo test -p sample"))]
    #[case::bash_command_with_parentheses("Bash(echo (sample))", "Bash", Some("echo (sample)"))]
    #[case::other_tool("Read(src/main.rs)", "Read(src/main.rs)", None)]
    #[case::bare_bash("Bash", "Bash", None)]
    fn pending_permission_fallback_parses_bash_command_without_reusing_the_last_assistant_message(
        #[case] current_tool: &str,
        #[case] expected_tool: &str,
        #[case] expected_command: Option<&str>,
    ) {
        let mut session = stored_session();
        session.pending_permission_agent_ids = BTreeSet::from([MAIN_THREAD_AGENT_KEY.to_string()]);
        session.pending_permission_request_ids = BTreeMap::from([(
            MAIN_THREAD_AGENT_KEY.to_string(),
            "permission-request-main".to_string(),
        )]);
        session.current_tool = Some(current_tool.to_string());
        session.last_message = Some("Run the package tests".to_string());

        assert_eq!(
            load_messages_with("fixture-session", |id| {
                Ok((id == "fixture-session").then(|| session.clone()))
            })
            .unwrap(),
            Some(vec![ChatItem::Permission {
                request_id: Some("permission-request-main".to_string()),
                tool: expected_tool.to_string(),
                description: "Waiting for permission".to_string(),
                command: expected_command.map(str::to_string),
            }])
        );
    }

    #[test]
    fn pending_permission_fallback_does_not_parse_a_shared_tool_during_sibling_work() {
        let mut session = stored_session();
        session.pending_permission_agent_ids = BTreeSet::from([MAIN_THREAD_AGENT_KEY.to_string()]);
        session.pending_agent_task_ids = BTreeSet::from(["fixture-agent".to_string()]);
        session.current_tool = Some("Bash(echo fixture)".to_string());

        assert_eq!(
            load_messages_with("fixture-session", |id| {
                Ok((id == "fixture-session").then(|| session.clone()))
            })
            .unwrap(),
            Some(vec![ChatItem::Permission {
                request_id: None,
                tool: "Bash(echo fixture)".to_string(),
                description: "Waiting for permission".to_string(),
                command: None,
            }])
        );
    }

    #[test]
    fn pending_permission_fallback_previews_oversized_bash_commands() {
        let mut session = stored_session();
        let oversized_command = "x".repeat(MAX_PERMISSION_COMMAND_BYTES + 1);
        session.pending_permission_agent_ids = BTreeSet::from([MAIN_THREAD_AGENT_KEY.to_string()]);
        session.current_tool = Some(format!("Bash({oversized_command})"));

        assert_eq!(
            load_messages_with("fixture-session", |id| {
                Ok((id == "fixture-session").then(|| session.clone()))
            })
            .unwrap(),
            Some(vec![ChatItem::Permission {
                request_id: None,
                tool: "Bash".to_string(),
                description: format!("Command preview: {}…", "x".repeat(200)),
                command: None,
            }])
        );
    }

    #[test]
    fn permission_card_serializes_its_request_id_for_watch_web_actions() {
        assert_eq!(
            serde_json::to_value(ChatItem::Permission {
                request_id: Some("permission-request-example".to_string()),
                tool: "Bash".to_string(),
                description: "Waiting for permission".to_string(),
                command: Some("cargo test -p sample".to_string()),
            })
            .unwrap(),
            json!({
                "type": "permission",
                "requestId": "permission-request-example",
                "tool": "Bash",
                "description": "Waiting for permission",
                "command": "cargo test -p sample",
            }),
        );
    }

    #[test]
    fn cancelled_question_serializes_its_cancellation_state() {
        assert_eq!(
            serde_json::to_value(ChatItem::Question {
                request_id: None,
                cancelled: true,
                questions: vec![QuestionPrompt {
                    header: None,
                    question: "Should the sample report include a chart?".to_string(),
                    choices: vec![question_choice("Yes")],
                    multi_select: false,
                    answer: None,
                }],
            })
            .unwrap(),
            json!({
                "type": "question",
                "cancelled": true,
                "questions": [{
                    "question": "Should the sample report include a chart?",
                    "choices": [{"label": "Yes"}],
                    "multi_select": false,
                    "answer": null,
                }],
            }),
        );
    }

    #[test]
    fn pending_question_does_not_create_a_permission_card() {
        let (_directory, transcript_path) = pending_question_transcript();

        let mut session = stored_session();
        session.pending_permission_agent_ids = BTreeSet::from([MAIN_THREAD_AGENT_KEY.to_string()]);
        session.pending_permission_request_ids = BTreeMap::from([(
            MAIN_THREAD_AGENT_KEY.to_string(),
            "permission-request-question".to_string(),
        )]);
        session.current_tool = Some("Bash(example subagent command)".to_string());
        session.transcript_path = Some(transcript_path);
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_permission_request_data_loader(|_, _| {
            Ok(Some(PermissionRequestData {
                tool_name: Some("AskUserQuestion".to_string()),
                tool_input: json!({
                    "questions": [{
                        "header": "Summary",
                        "question": "Should the summary be brief?",
                        "options": [{ "label": "Yes" }, { "label": "No" }],
                        "multiSelect": true,
                    }],
                }),
                pending: true,
            }))
        });

        assert_eq!(
            load_conversation_with_queued_prompts(
                "fixture-session",
                |id| Ok((id == "fixture-session").then(|| session.clone())),
                |_| Ok(None),
                &loaders,
            )
            .unwrap()
            .map(|conversation| conversation.messages),
            Some(vec![ChatItem::Question {
                request_id: Some("permission-request-question".to_string()),
                cancelled: false,
                questions: vec![QuestionPrompt {
                    header: Some("Summary".to_string()),
                    question: "Should the summary be brief?".to_string(),
                    choices: vec![
                        QuestionChoice {
                            label: "Yes".to_string(),
                            description: Some("Keep it concise.".to_string()),
                        },
                        question_choice("No"),
                    ],
                    multi_select: true,
                    answer: None,
                }],
            }])
        );
    }

    #[test]
    fn subagent_permission_does_not_reuse_main_question_details() {
        let (_directory, transcript_path) = pending_question_transcript();
        let mut session = stored_session();
        session.pending_permission_agent_ids = BTreeSet::from([
            MAIN_THREAD_AGENT_KEY.to_string(),
            "fixture-agent-id".to_string(),
        ]);
        session.pending_permission_request_ids = BTreeMap::from([
            (
                MAIN_THREAD_AGENT_KEY.to_string(),
                "permission-request-main".to_string(),
            ),
            (
                "fixture-agent-id".to_string(),
                "permission-request-subagent".to_string(),
            ),
        ]);
        session.current_tool = Some("AskUserQuestion".to_string());
        session.current_tool_name = Some("AskUserQuestion".to_string());
        session.last_message = Some("Should the summary be brief?".to_string());
        session.transcript_path = Some(transcript_path);

        assert_eq!(
            load_messages_with("fixture-session", |id| {
                Ok((id == "fixture-session").then(|| session.clone()))
            })
            .unwrap(),
            Some(vec![
                ChatItem::Question {
                    request_id: None,
                    cancelled: false,
                    questions: vec![QuestionPrompt {
                        header: Some("Summary".to_string()),
                        question: "Should the summary be brief?".to_string(),
                        choices: vec![
                            QuestionChoice {
                                label: "Yes".to_string(),
                                description: Some("Keep it concise.".to_string()),
                            },
                            question_choice("No"),
                        ],
                        multi_select: true,
                        answer: None,
                    }],
                },
                ChatItem::Permission {
                    request_id: Some("permission-request-subagent".to_string()),
                    tool: "Subagent fixture-agent-id: Permission required".to_string(),
                    description: "Waiting for permission approval.".to_string(),
                    command: None,
                },
            ])
        );
    }

    #[rstest]
    #[case::within_highlight_limit(false)]
    #[case::oversized_command(true)]
    #[test]
    fn pending_permission_cards_target_each_agent_and_omit_question_requests(
        #[case] oversized_command: bool,
    ) {
        let command = if oversized_command {
            format!("echo {}", "x".repeat(MAX_PERMISSION_COMMAND_BYTES))
        } else {
            "printf 'preview'\ngit status --short".to_string()
        };
        let expected_command = (!oversized_command).then(|| command.clone());
        let expected_description = if oversized_command {
            let preview = command.chars().take(200).collect::<String>();
            format!("Command preview: {preview}…")
        } else {
            "Waiting for permission".to_string()
        };
        let mut session = stored_session();
        session.pending_permission_agent_ids = BTreeSet::from([
            "agent-fixture-a".to_string(),
            "agent-fixture-question".to_string(),
            MAIN_THREAD_AGENT_KEY.to_string(),
        ]);
        session.pending_permission_request_ids = BTreeMap::from([
            (
                "agent-fixture-a".to_string(),
                "permission-request-a".to_string(),
            ),
            (
                "agent-fixture-question".to_string(),
                "permission-request-question".to_string(),
            ),
            (
                MAIN_THREAD_AGENT_KEY.to_string(),
                "permission-request-main".to_string(),
            ),
        ]);
        session.current_tool = Some("Bash(example command)".to_string());
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_permission_request_data_loader(move |_, request_id| {
            let (tool_name, tool_input) = match request_id {
                "permission-request-main" => ("Bash", json!({ "command": command.clone() })),
                "permission-request-a" => ("Write", json!({})),
                "permission-request-question" => (
                    "AskUserQuestion",
                    json!({
                        "questions": [{
                            "header": "Summary",
                            "question": "Should the summary be brief?",
                            "options": [{ "label": "Yes" }, { "label": "No" }],
                            "multiSelect": true,
                        }],
                    }),
                ),
                _ => return Ok(None),
            };
            Ok(Some(PermissionRequestData {
                tool_name: Some(tool_name.to_string()),
                tool_input,
                pending: true,
            }))
        });

        let actual = load_conversation_with_queued_prompts(
            "fixture-session",
            |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            &loaders,
        )
        .unwrap()
        .unwrap()
        .messages;

        assert_eq!(
            actual,
            vec![
                ChatItem::Permission {
                    request_id: Some("permission-request-main".to_string()),
                    tool: "Bash".to_string(),
                    description: expected_description,
                    command: expected_command,
                },
                ChatItem::Permission {
                    request_id: Some("permission-request-a".to_string()),
                    tool: "Subagent agent-fixture-a: Write".to_string(),
                    description: "Waiting for permission approval.".to_string(),
                    command: None,
                },
                ChatItem::Question {
                    request_id: Some("permission-request-question".to_string()),
                    cancelled: false,
                    questions: vec![QuestionPrompt {
                        header: Some("Summary".to_string()),
                        question: "Should the summary be brief?".to_string(),
                        choices: vec![question_choice("Yes"), question_choice("No")],
                        multi_select: true,
                        answer: None,
                    }],
                },
            ],
        );
    }

    #[rstest]
    #[case::plain(false, false, "Subagent fixture-agent: Bash(run fixture)")]
    #[case::compressed(true, false, "Subagent fixture-agent: Bash(run fixture)")]
    #[case::malformed_compressed(true, true, "Subagent fixture-agent: Bash")]
    fn subagent_permission_matches_an_unresolved_tool_use_from_its_transcript(
        #[case] compressed: bool,
        #[case] malformed: bool,
        #[case] expected_tool: &str,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let transcript_suffix = if compressed { ".jsonl.zst" } else { ".jsonl" };
        let transcript_path = directory
            .path()
            .join(format!("fixture-session{transcript_suffix}"));
        let parent_transcript = if compressed {
            zstd::stream::encode_all(&b""[..], 0).unwrap()
        } else {
            Vec::new()
        };
        std::fs::write(&transcript_path, parent_transcript).unwrap();
        let subagent_directory = directory.path().join("fixture-session").join("subagents");
        std::fs::create_dir_all(&subagent_directory).unwrap();
        let subagent_transcript = indoc::indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"first-call","name":"Bash","input":{"command":"run fixture"}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"first-call","content":"done"}]}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"pending-call","name":"Bash","input":{"command":"run fixture"}}]}}
        "#};
        let subagent_transcript = if malformed {
            b"invalid compressed transcript".to_vec()
        } else if compressed {
            zstd::stream::encode_all(subagent_transcript.as_bytes(), 0).unwrap()
        } else {
            subagent_transcript.as_bytes().to_vec()
        };
        std::fs::write(
            subagent_directory.join(format!("agent-fixture-agent{transcript_suffix}")),
            subagent_transcript,
        )
        .unwrap();

        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        session.pending_permission_agent_ids = BTreeSet::from(["fixture-agent".to_string()]);
        session.pending_permission_request_ids = BTreeMap::from([(
            "fixture-agent".to_string(),
            "permission-request-subagent".to_string(),
        )]);
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_permission_request_data_loader(|_, _| {
            Ok(Some(PermissionRequestData {
                tool_name: Some("Bash".to_string()),
                tool_input: json!({"command": "run fixture"}),
                pending: true,
            }))
        });

        assert_eq!(
            load_conversation_with_queued_prompts(
                "fixture-session",
                |id| Ok((id == "fixture-session").then(|| session.clone())),
                |_| Ok(None),
                &loaders,
            )
            .unwrap()
            .map(|conversation| conversation.messages),
            Some(vec![ChatItem::Permission {
                request_id: Some("permission-request-subagent".to_string()),
                tool: expected_tool.to_string(),
                description: "Waiting for permission approval.".to_string(),
                command: Some("run fixture".to_string()),
            }]),
        );
    }

    #[tokio::test]
    async fn peer_message_route_includes_a_resolved_sender_title() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let record = json!({
            "type": "user",
            "message": {
                "content": concat!(
                    "<peer-message>", "\n",
                    "- From session_id: fixture-parent-id", "\n", "\n",
                    "Synthetic peer request.", "\n",
                    "</peer-message>",
                ),
            },
        });
        std::fs::write(&transcript_path, serde_json::to_vec(&record).unwrap()).unwrap();

        let mut child = stored_session();
        child.session_id = "fixture-child-id".to_string();
        child.transcript_path = Some(transcript_path);
        let mut sender = stored_session();
        sender.session_id = "fixture-parent-id".to_string();
        sender.label = Some("Fixture parent".to_string());

        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |id| {
                Ok(match id {
                    "fixture-child-id" => Some(child.clone()),
                    "fixture-parent-id" => Some(sender.clone()),
                    _ => None,
                })
            },
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-child-id/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response_snapshot(response).await,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                r#"{"messages":[{"type":"peer","direction":"from","session_id":"fixture-parent-id","session_title":"Fixture parent","content":[{"type":"text","text":"Synthetic peer request."}]}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null}"#
                    .to_string(),
            )
        );
    }

    #[tokio::test]
    async fn peer_title_resolution_refreshes_a_cached_conversation() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let record = json!({
            "type": "user",
            "message": {
                "content": concat!(
                    "<peer-message>", "\n",
                    "- From session_id: fixture-sender-id", "\n", "\n",
                    "Synthetic peer request.", "\n",
                    "</peer-message>",
                ),
            },
        });
        std::fs::write(&transcript_path, serde_json::to_vec(&record).unwrap()).unwrap();

        let mut child = stored_session();
        child.transcript_path = Some(transcript_path);
        let mut sender = stored_session();
        sender.session_id = "fixture-sender-id".to_string();
        sender.label = Some("Fixture sender".to_string());
        let sender_is_available = Arc::new(AtomicBool::new(false));
        let sender_availability = Arc::clone(&sender_is_available);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |id| {
                Ok(match id {
                    "fixture-session" => Some(child.clone()),
                    "fixture-sender-id" if sender_availability.load(Ordering::SeqCst) => {
                        Some(sender.clone())
                    }
                    _ => None,
                })
            },
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );

        let without_sender = message_response(&router).await;
        sender_is_available.store(true, Ordering::SeqCst);
        let with_sender = message_response(&router).await;

        assert_eq!(
            (without_sender, with_sender),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({
                        "messages": [{
                            "type": "peer",
                            "direction": "from",
                            "session_id": "fixture-sender-id",
                            "content": [{"type": "text", "text": "Synthetic peer request."}],
                        }],
                        "backgroundTasks": [],
                        "start": 0,
                        "total": 1,
                        "decisions": [],
                        "hasUnansweredProposal": false,
                        "activeQuestionIndex": null,
                    }),
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({
                        "messages": [{
                            "type": "peer",
                            "direction": "from",
                            "session_id": "fixture-sender-id",
                            "session_title": "Fixture sender",
                            "content": [{"type": "text", "text": "Synthetic peer request."}],
                        }],
                        "backgroundTasks": [],
                        "start": 0,
                        "total": 1,
                        "decisions": [],
                        "hasUnansweredProposal": false,
                        "activeQuestionIndex": null,
                    }),
                ),
            ),
        );
    }

    #[tokio::test]
    async fn peer_message_route_ignores_malformed_sender_ids() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let record = json!({
            "type": "user",
            "message": {
                "content": concat!(
                    "<peer-message>", "\n",
                    "- From session_id: ../fixture-id", "\n", "\n",
                    "Synthetic peer request.", "\n",
                    "</peer-message>",
                ),
            },
        });
        std::fs::write(&transcript_path, serde_json::to_vec(&record).unwrap()).unwrap();

        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );

        assert_eq!(
            message_response(&router).await,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                json!({
                    "messages": [{
                        "type": "peer",
                        "direction": "from",
                        "session_id": "../fixture-id",
                        "content": [{"type": "text", "text": "Synthetic peer request."}],
                    }],
                    "backgroundTasks": [],
                    "start": 0,
                    "total": 1,
                    "decisions": [],
                    "hasUnansweredProposal": false,
                    "activeQuestionIndex": null,
                }),
            ),
        );
    }

    #[rstest]
    #[case::delivered(false)]
    #[case::delivery_failed(true)]
    #[tokio::test]
    async fn peer_out_route_renders_the_database_delivery_result(#[case] failed: bool) {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let transcript = indoc::indoc! {r#"
            {"type":"assistant","timestamp":"2026-06-01T12:00:00Z","message":{"content":[{"type":"tool_use","id":"fixture-peer-notify-call","name":"Bash","input":{"command":"cargo check --example sample && a agent peer notify -m 'Synthetic request.' fixture-target-id"}}]}}
            {"type":"user","timestamp":"2026-06-01T12:00:02Z","sourceToolAssistantUUID":"fixture-assistant-call","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-peer-notify-call","content":"0 failed"}]}}
        "#};
        std::fs::write(&transcript_path, transcript).unwrap();

        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let sent_at = chrono::DateTime::parse_from_rfc3339("2026-06-01T12:00:01Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            move |from| {
                Ok(if from == "fixture-session" {
                    vec![WatchWebPeerOutMessage {
                        id: 1,
                        from_session_id: "fixture-session".to_string(),
                        to_session_id: "fixture-target-id".to_string(),
                        content: "Expanded fictional request body.".to_string(),
                        sent_at,
                        failed,
                    }]
                } else {
                    Vec::new()
                })
            },
            |_, _| Ok(()),
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let (status, content_type, body) = response_snapshot(response).await;
        let mut expected_message = json!({
            "type": "peer",
            "direction": "to",
            "session_id": "fixture-target-id",
            "content": [{"type": "text", "text": "Expanded fictional request body."}],
            "timestamp": "2026-06-01T12:00:01+00:00",
        });
        if failed {
            expected_message["failed"] = json!(true);
        }

        assert_eq!(
            (
                status,
                content_type,
                serde_json::from_str::<serde_json::Value>(&body).unwrap(),
            ),
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                json!({
                    "messages": [
                        expected_message,
                        {
                            "type": "tool_summary",
                            "summary": "Bash ×1",
                            "operations": [{
                                "tool": "Bash",
                                "target": "cargo check --example sample && a agent peer notify -m 'Synthetic request.' fixture-target-id",
                                "failed": false,
                            }],
                        },
                    ],
                    "backgroundTasks": [],
                    "start": 0,
                    "total": 2,
                    "decisions": [],
                    "hasUnansweredProposal": false,
                    "activeQuestionIndex": null,
                }),
            ),
        );
    }

    #[tokio::test]
    async fn codex_message_route_places_database_peer_out_by_send_time() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let transcript = indoc::indoc! {r#"
            {"timestamp":"2026-06-01T12:00:00Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Before delivery."}]}}
            {"timestamp":"2026-06-01T12:00:02Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"After delivery."}]}}
        "#};
        std::fs::write(&transcript_path, transcript).unwrap();

        let mut session = stored_session();
        session.engine = Engine::Codex;
        session.transcript_path = Some(transcript_path);
        let sent_at = chrono::DateTime::parse_from_rfc3339("2026-06-01T12:00:01Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            move |_| {
                Ok(vec![WatchWebPeerOutMessage {
                    id: 1,
                    from_session_id: "fixture-session".to_string(),
                    to_session_id: "fixture-target-id".to_string(),
                    content: "The fictional request was not delivered.".to_string(),
                    sent_at,
                    failed: true,
                }])
            },
            |_, _| Ok(()),
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, content_type, body) = response_snapshot(response).await;

        assert_eq!(
            (
                status,
                content_type,
                serde_json::from_str::<serde_json::Value>(&body).unwrap(),
            ),
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                json!({
                    "messages": [
                        {"type": "assistant", "text": "Before delivery."},
                        {
                            "type": "peer",
                            "direction": "to",
                            "session_id": "fixture-target-id",
                            "content": [{"type": "text", "text": "The fictional request was not delivered."}],
                            "timestamp": "2026-06-01T12:00:01+00:00",
                            "failed": true,
                        },
                        {"type": "assistant", "text": "After delivery."},
                    ],
                    "backgroundTasks": [],
                    "start": 0,
                    "total": 3,
                    "decisions": [],
                    "hasUnansweredProposal": false,
                    "activeQuestionIndex": null,
                }),
            ),
        );
    }

    #[tokio::test]
    async fn question_message_route_serializes_question_metadata() {
        let (_directory, transcript_path) = pending_question_transcript();
        let mut session = stored_session();
        session.current_tool = Some("AskUserQuestion".to_string());
        session.current_tool_name = Some("AskUserQuestion".to_string());
        session.transcript_path = Some(transcript_path);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response).await,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                r#"{"messages":[{"type":"question","questions":[{"header":"Summary","question":"Should the summary be brief?","choices":[{"label":"Yes","description":"Keep it concise."},{"label":"No"}],"multi_select":true,"answer":null}]}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":0}"#
                    .to_string(),
            )
        );
    }

    #[tokio::test]
    async fn claude_message_route_returns_contentful_transcript_queue_items() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            br#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-06-01T12:00:00Z","content":"Continue the fixture build."}"#,
        )
        .unwrap();

        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response_snapshot(response).await,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                r#"{"messages":[{"type":"queued_user","content":[{"type":"text","text":"Continue the fixture build."}],"timestamp":"2026-06-01T12:00:00+00:00"}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null}"#.to_string(),
            ),
        );
    }

    #[tokio::test]
    async fn message_route_confirms_only_requested_resolved_prompt_ids() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            br#"{"type":"queue-operation","operation":"popAll","timestamp":"2026-01-01T00:00:01Z"}"#,
        )
        .unwrap();

        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let mut already_resolved = saved_prompt(8, "An unrelated resolved prompt.");
        already_resolved.resolved = true;
        let mut still_pending = saved_prompt(9, "A later queued prompt.");
        still_pending.sent_at = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:02Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let queued_prompts = Arc::new(Mutex::new(vec![
            saved_prompt(7, "A cancelled prompt."),
            already_resolved,
            still_pending,
        ]));
        let queue_loader = Arc::clone(&queued_prompts);
        let queue_updater = Arc::clone(&queued_prompts);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            move |_| Ok(queue_loader.lock().unwrap().clone()),
            |_| Ok(Vec::new()),
            move |_, ids| {
                for prompt in queue_updater.lock().unwrap().iter_mut() {
                    if ids.contains(&prompt.id) {
                        prompt.resolved = true;
                    }
                }
                Ok(())
            },
        );
        let query = "?pendingPromptIds=fixture-message-7%2Cfixture-message-9";

        let first = message_response_with_query(&router, query).await;
        let second = message_response_with_query(&router, query).await;
        let third = message_response_with_query(&router, query).await;
        let persisted_resolution = queued_prompts
            .lock()
            .unwrap()
            .iter()
            .map(|prompt| (prompt.id, prompt.resolved))
            .collect::<Vec<_>>();
        let expected_response = (
            StatusCode::OK,
            Some("application/json".to_string()),
            json!({
                "messages": [{
                    "type": "queued_user",
                    "content": [{"type": "text", "text": "A later queued prompt."}],
                    "queuedPromptId": "fixture-message-9",
                    "timestamp": "2026-01-01T00:00:02+00:00",
                }],
                "confirmedQueuedPromptIds": ["fixture-message-7"],
                "backgroundTasks": [],
                "start": 0,
                "total": 1,
                "decisions": [],
                "hasUnansweredProposal": false,
                "activeQuestionIndex": null,
            }),
        );

        assert_eq!(
            (vec![first, second, third], persisted_resolution),
            (
                vec![expected_response.clone(); 3],
                vec![(7, true), (8, true), (9, false)],
            ),
        );
    }

    #[tokio::test]
    async fn claude_message_route_returns_saved_prompts_when_no_transcript_exists() {
        let session = stored_session();
        let queued = saved_prompt(7, "Continue the fixture build.");
        let deleted_ids = Arc::new(Mutex::new(Vec::new()));
        let delete_log = Arc::clone(&deleted_ids);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(Some("⟦armyknife-user:fixture-marker-cobalt⟧".to_string())),
            |_| Ok(Vec::new()),
            move |_| Ok(vec![queued.clone()]),
            |_| Ok(Vec::new()),
            move |_, ids| {
                delete_log.lock().unwrap().extend_from_slice(ids);
                Ok(())
            },
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                response_snapshot(response).await,
                deleted_ids.lock().unwrap().clone(),
            ),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                r#"{"messages":[{"type":"queued_user","content":[{"type":"text","text":"Continue the fixture build."}],"queuedPromptId":"fixture-message-7","timestamp":"2026-01-01T00:00:00+00:00"}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null}"#.to_string(),
                ),
                Vec::<i64>::new(),
            ),
        );
    }

    #[tokio::test]
    async fn claude_message_route_marks_prompts_resolved_after_id_delivery() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            r#"{"type":"user","origin":{"kind":"peer","from":"unknown"},"timestamp":"2026-01-01T00:00:01Z","message":{"content":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nEngine-formatted delivery.\n<!-- armyknife-queued-prompt-id:fixture-message-7 -->"}}"#,
        )
        .unwrap();
        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let queued = saved_prompt(7, "Delivered fixture request.");
        let marked_ids = Arc::new(Mutex::new(Vec::new()));
        let mark_log = Arc::clone(&marked_ids);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(Some("⟦armyknife-user:fixture-marker-cobalt⟧".to_string())),
            |_| Ok(Vec::new()),
            move |_| Ok(vec![queued.clone()]),
            |_| Ok(Vec::new()),
            move |_, ids| {
                mark_log.lock().unwrap().extend_from_slice(ids);
                Ok(())
            },
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                response_snapshot(response).await,
                marked_ids.lock().unwrap().clone(),
            ),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                r#"{"messages":[{"type":"user","text":"Delivered fixture request.","queuedPromptId":"fixture-message-7","timestamp":"2026-01-01T00:00:01+00:00"}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null}"#.to_string(),
                ),
                vec![7],
            ),
        );
    }

    #[rstest]
    #[case::active_delivery(true, 0, 205)]
    #[case::abandoned_delivery(false, 0, 205)]
    #[case::rewound_history(false, 205, 205)]
    #[tokio::test]
    async fn initial_tail_window_matches_full_parse_for_queued_prompt_branches(
        #[case] delivery_is_active: bool,
        #[case] abandoned_input_count: usize,
        #[case] active_input_count: usize,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let delivery_content = indoc::indoc! {r#"
            ⟦armyknife-user:fixture-marker-cobalt⟧
            This message was typed by the human user in armyknife watch-web.
            Delivered fixture request.
            <!-- armyknife-queued-prompt-id:fixture-message-7 -->
        "#};
        let mut records = vec![
            json!({
                "type": "system",
                "padding": "x".repeat(5 * 1024 * 1024),
            }),
            json!({
                "type": "user",
                "uuid": "fixture-root",
                "parentUuid": null,
                "timestamp": "2026-01-01T00:00:01Z",
                "message": {"content": "Fixture branch root."},
            }),
            json!({
                "type": "user",
                "uuid": "fixture-delivery",
                "parentUuid": "fixture-root",
                "timestamp": "2026-01-01T00:00:01Z",
                "origin": {"kind": "peer", "from": "fixture-peer"},
                "message": {"content": delivery_content},
            }),
        ];
        let mut parent_uuid = "fixture-delivery".to_owned();
        for index in 0..abandoned_input_count {
            let uuid = format!("fixture-abandoned-request-{index}");
            records.push(json!({
                "type": "user",
                "uuid": uuid,
                "parentUuid": parent_uuid,
                "timestamp": "2026-01-01T00:00:01Z",
                "message": {"content": format!("Abandoned fixture request {index}.")},
            }));
            parent_uuid = uuid;
        }
        if !delivery_is_active {
            records.push(json!({
                "type": "assistant",
                "uuid": "fixture-current-branch",
                "parentUuid": "fixture-root",
                "timestamp": "2026-01-01T00:00:01Z",
                "message": {"content": [{"type": "text", "text": "Current fixture branch."}]},
            }));
            parent_uuid = "fixture-current-branch".to_owned();
        }
        for index in 0..active_input_count {
            let uuid = format!("fixture-request-{index}");
            records.push(json!({
                "type": "user",
                "uuid": uuid,
                "parentUuid": parent_uuid,
                "timestamp": "2026-01-01T00:00:01Z",
                "message": {"content": format!("Fixture request {index}.")},
            }));
            parent_uuid = uuid;
        }
        let transcript = records
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&transcript_path, &transcript).unwrap();
        let compressed_path = directory.path().join("full-transcript.jsonl.zst");
        std::fs::write(
            &compressed_path,
            zstd::stream::encode_all(transcript.as_bytes(), 0).unwrap(),
        )
        .unwrap();
        let base_session = stored_session();
        let queued = saved_prompt(7, "Delivered fixture request.");
        let old_created_at = chrono::DateTime::parse_from_rfc3339("2025-12-31T23:59:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let pending_decision = AgentDecision {
            session_id: "fixture-session".to_string(),
            seq: 1,
            title: "Old pending decision".to_string(),
            why: "The fixture keeps this decision pending.".to_string(),
            considered: Vec::new(),
            easy_to_change: "Change one fixture value.".to_string(),
            created_at: old_created_at.to_rfc3339(),
            review: None,
            reviewed_at: None,
        };
        let pending_proposal = AgentProposal {
            session_id: "fixture-session".to_string(),
            seq: 1,
            body: r#"{"title":"Old pending proposal","summary":null,"options":[{"label":"Option A"},{"label":"Option B"}]}"#.to_string(),
            created_at: old_created_at,
            answer: None,
            resolved_at: None,
            resolution: None,
            comment: None,
        };
        let pending_todo = AgentTodo {
            session_id: "fixture-session".to_string(),
            seq: 1,
            title: "Old pending todo".to_string(),
            detail: None,
            folded: false,
            added_folded: false,
            created_at: old_created_at,
            completed_at: None,
            resolution: None,
            comment: None,
        };
        let pending_denial = PermissionDeniedOperation {
            session_id: "fixture-session".to_string(),
            seq: 1,
            tool_use_id: Some("fixture-old-pending-denial".to_string()),
            tool_name: "Bash".to_string(),
            tool_input: json!({"command": "echo fixture"}),
            reason: Some("Fixture denial".to_string()),
            created_at: old_created_at,
            resolved_at: None,
            resolution: None,
        };
        let full_marked_ids = Arc::new(Mutex::new(Vec::new()));
        let tail_marked_ids = Arc::new(Mutex::new(Vec::new()));
        let make_router = |marked_ids: Arc<Mutex<Vec<i64>>>, transcript_path: &Path| {
            let mut session = base_session.clone();
            session.transcript_path = Some(transcript_path.to_path_buf());
            let queued = queued.clone();
            let pending_decision = pending_decision.clone();
            let pending_proposal = pending_proposal.clone();
            let pending_todo = pending_todo.clone();
            let pending_denial = pending_denial.clone();
            let loaders = RouteLoaders::new(
                move |_| Ok(vec![pending_decision.clone()]),
                move |_| Ok(vec![queued.clone()]),
                |_| Ok(Vec::new()),
                move |_, ids| {
                    marked_ids.lock().unwrap().extend_from_slice(ids);
                    Ok(())
                },
                move |_| Ok(vec![pending_proposal.clone()]),
            )
            .with_todo_loader(move |_| Ok(vec![pending_todo.clone()]))
            .with_permission_denied_loader(move |_| Ok(vec![pending_denial.clone()]));
            let cache = MessageCache::default();
            let mut loaders = loaders;
            loaders.message_cache = cache.clone();
            let router = routes_with_proposals_and_peer_out_messages(
                move |_| Ok(Some(session.clone())),
                |_| Ok(Some("⟦armyknife-user:fixture-marker-cobalt⟧".to_string())),
                loaders,
            );
            (router, cache)
        };
        let (full_router, _) = make_router(Arc::clone(&full_marked_ids), &compressed_path);
        let (tail_router, tail_cache) = make_router(Arc::clone(&tail_marked_ids), &transcript_path);
        let full_initial = message_response_with_query(&full_router, "?limit=100").await;
        let tail_initial = message_response_with_query(&tail_router, "?limit=100").await;
        let mut full_responses = vec![full_initial];
        let mut tail_responses = vec![tail_initial.clone()];
        let epoch = tail_initial.2["epoch"].as_str().unwrap().to_owned();
        let older_cursor = tail_initial.2["olderCursor"].as_str().unwrap();
        let start = tail_initial.2["start"].as_u64().unwrap();
        let mut refresh_query = format!("?from={start}&epoch={epoch}&olderCursor={older_cursor}");
        if let (Some(stable_start), Some(stable_end), Some(stable_digest)) = (
            tail_initial.2["stableStart"].as_u64(),
            tail_initial.2["stableEnd"].as_u64(),
            tail_initial.2["stableDigest"].as_str(),
        ) {
            refresh_query.push_str(&format!(
                "&stableStart={stable_start}&stableEnd={stable_end}&stableDigest={stable_digest}"
            ));
        }
        let last_uuid = records
            .last()
            .and_then(|record| record["uuid"].as_str())
            .unwrap()
            .to_owned();
        let appended_line = format!(
            "{{\"type\":\"user\",\"uuid\":\"fixture-appended-request\",\"parentUuid\":\"{last_uuid}\",\"timestamp\":\"2026-01-01T00:00:02Z\",\"message\":{{\"content\":\"Appended active fixture request.\"}}}}"
        );
        let partial_transcript = format!(
            "{}{}",
            transcript,
            &appended_line[..appended_line.len() / 2]
        );
        std::fs::write(&transcript_path, &partial_transcript).unwrap();
        std::fs::write(
            &compressed_path,
            zstd::stream::encode_all(partial_transcript.as_bytes(), 0).unwrap(),
        )
        .unwrap();
        full_responses.push(message_response_with_query(&full_router, &refresh_query).await);
        tail_responses.push(message_response_with_query(&tail_router, &refresh_query).await);

        let complete_transcript = format!(
            "{partial_transcript}{}\n",
            &appended_line[appended_line.len() / 2..]
        );
        std::fs::write(&transcript_path, &complete_transcript).unwrap();
        std::fs::write(
            &compressed_path,
            zstd::stream::encode_all(complete_transcript.as_bytes(), 0).unwrap(),
        )
        .unwrap();

        full_responses.push(message_response_with_query(&full_router, &refresh_query).await);
        tail_responses.push(message_response_with_query(&tail_router, &refresh_query).await);
        let parser_slot = tail_cache.transcript_parser_slot("fixture-session");
        let cached_parser = parser_slot
            .lock()
            .unwrap()
            .as_ref()
            .map(|entry| (entry.last_change, entry.tail_window.is_some()));
        let refreshed_epoch = tail_responses[2].2["epoch"].as_str().map(str::to_owned);
        let refreshed_older_cursor = tail_responses[2].2["olderCursor"]
            .as_str()
            .map(str::to_owned);
        let refresh_reset = tail_responses[2].2["epochReset"].as_bool().unwrap_or(false);
        let older_query = format!(
            "?limit=100&epoch={epoch}&olderCursor={}",
            refreshed_older_cursor.as_deref().unwrap_or(older_cursor)
        );
        let tail_parser_before_older_page = parser_slot
            .lock()
            .unwrap()
            .as_ref()
            .map(|entry| entry.tail_window.is_some());
        full_responses.push(message_response_with_query(&full_router, &older_query).await);
        tail_responses.push(message_response_with_query(&tail_router, &older_query).await);
        let older_reset = tail_responses[3].2["epochReset"].as_bool().unwrap_or(false);
        let older_message_count = tail_responses[3].2["messages"].as_array().map(Vec::len);
        let tail_parser_after_older_page = parser_slot
            .lock()
            .unwrap()
            .as_ref()
            .map(|entry| entry.tail_window.is_some());
        let full_marked_ids = full_marked_ids.lock().unwrap().clone();
        let tail_marked_ids = tail_marked_ids.lock().unwrap().clone();
        assert_eq!(
            (
                full_responses,
                tail_responses.clone(),
                full_marked_ids.clone(),
                tail_marked_ids,
                refreshed_epoch,
                refreshed_older_cursor.is_some(),
                refresh_reset,
                cached_parser,
                tail_parser_before_older_page,
                tail_parser_after_older_page,
                older_reset,
                older_message_count,
            ),
            (
                tail_responses.clone(),
                tail_responses.clone(),
                full_marked_ids.clone(),
                full_marked_ids,
                Some(epoch),
                true,
                false,
                Some((TranscriptChange::Appended, true)),
                Some(true),
                Some(true),
                false,
                Some(100),
            ),
        );
    }

    #[rstest]
    #[case::unbounded_history("")]
    #[case::numeric_older_page("?limit=100&before=100")]
    #[tokio::test]
    async fn cached_tail_window_falls_back_for_full_history_requests(#[case] query: &str) {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let compressed_path = directory.path().join("transcript.jsonl.zst");
        let mut records = vec![json!({
            "type": "system",
            "padding": "x".repeat(5 * 1024 * 1024),
        })];
        let mut parent_uuid = None;
        for index in 0..120 {
            let uuid = format!("fixture-message-{index}");
            records.push(json!({
                "type": "user",
                "uuid": uuid,
                "parentUuid": parent_uuid,
                "timestamp": "2026-01-01T00:00:01Z",
                "message": {"content": format!("Fixture request {index}.")},
            }));
            parent_uuid = Some(uuid);
        }
        let transcript = records
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&transcript_path, &transcript).unwrap();
        std::fs::write(
            &compressed_path,
            zstd::stream::encode_all(transcript.as_bytes(), 0).unwrap(),
        )
        .unwrap();

        let make_router = |path: &Path| {
            let mut session = stored_session();
            session.transcript_path = Some(path.to_path_buf());
            let cache = MessageCache::default();
            let mut loaders = empty_route_loaders();
            loaders.message_cache = cache.clone();
            let router = routes_with_proposals_and_peer_out_messages(
                move |_| Ok(Some(session.clone())),
                |_| Ok(None),
                loaders,
            );
            (router, cache)
        };
        let (full_router, _) = make_router(&compressed_path);
        let (tail_router, tail_cache) = make_router(&transcript_path);
        let initial = message_response_with_query(&tail_router, "?limit=100").await;
        let full_response = message_response_with_query(&full_router, query).await;
        let tail_response = message_response_with_query(&tail_router, query).await;
        let tail_window_cached = tail_cache
            .transcript_parser_slot("fixture-session")
            .lock()
            .unwrap()
            .as_ref()
            .map(|entry| entry.tail_window.is_some());

        assert_eq!(
            (initial.0, tail_response, tail_window_cached),
            (StatusCode::OK, full_response, Some(false)),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn older_epoch_pages_expand_and_reuse_the_tail_window(
        #[with(400)] older_page_fixture: OlderPageFixture,
    ) {
        let OlderPageFixture {
            transcript,
            transcript_path,
            compressed_path,
            full_router,
            tail_router,
            tail_cache,
            ..
        } = older_page_fixture;
        let full_initial = message_response_with_query(&full_router, "?limit=100").await;
        let tail_initial = message_response_with_query(&tail_router, "?limit=100").await;
        let epoch = tail_initial.2["epoch"].as_str().unwrap();
        let cursor = tail_initial.2["olderCursor"].as_str().unwrap();
        let older_query = format!("?limit=100&epoch={epoch}&olderCursor={cursor}");
        let full_older = message_response_with_query(&full_router, &older_query).await;
        let tail_older = message_response_with_query(&tail_router, &older_query).await;
        let older_epoch = tail_older.2["epoch"].as_str().unwrap().to_owned();
        let older_cursor = tail_older.2["olderCursor"].as_str().unwrap().to_owned();
        let next_older_query = format!("?limit=100&epoch={older_epoch}&olderCursor={older_cursor}");
        let full_older_again = message_response_with_query(&full_router, &next_older_query).await;
        let tail_older_again = message_response_with_query(&tail_router, &next_older_query).await;
        let page_epoch = tail_older_again.2["epoch"].as_str().unwrap().to_owned();
        let page_cursor = tail_older_again.2["olderCursor"]
            .as_str()
            .unwrap()
            .to_owned();
        let page_start = tail_older_again.2["start"].as_u64().unwrap();
        let refresh_query =
            format!("?from={page_start}&epoch={page_epoch}&olderCursor={page_cursor}");
        let full_refresh = message_response_with_query(&full_router, &refresh_query).await;
        let tail_refresh = message_response_with_query(&tail_router, &refresh_query).await;

        let appended = json!({
            "type": "user",
            "uuid": "fixture-message-400",
            "parentUuid": "fixture-message-399",
            "timestamp": "2026-01-01T00:00:02Z",
            "message": {"content": "Fixture appended request."},
        })
        .to_string();
        let updated_transcript = format!("{transcript}{appended}\n");
        std::fs::write(&transcript_path, &updated_transcript).unwrap();
        std::fs::write(
            &compressed_path,
            zstd::stream::encode_all(updated_transcript.as_bytes(), 0).unwrap(),
        )
        .unwrap();
        let full_after_append = message_response_with_query(&full_router, &refresh_query).await;
        let tail_after_append = message_response_with_query(&tail_router, &refresh_query).await;
        let parser_snapshot = tail_cache
            .transcript_parser_slot("fixture-session")
            .lock()
            .unwrap()
            .as_ref()
            .map(|entry| {
                (
                    entry.last_change,
                    entry.tail_window.as_ref().map(|state| state.message_limit),
                )
            });

        assert_eq!(
            (
                tail_initial,
                tail_older,
                tail_older_again,
                tail_refresh,
                tail_after_append,
                parser_snapshot,
            ),
            (
                full_initial,
                full_older,
                full_older_again,
                full_refresh,
                full_after_append,
                Some((TranscriptChange::Appended, Some(300))),
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn older_epoch_pages_expand_geometrically_for_stale_cursors(
        #[with(1_000)] older_page_fixture: OlderPageFixture,
    ) {
        let OlderPageFixture {
            full_router,
            tail_router,
            tail_cache,
            ..
        } = older_page_fixture;
        let full_initial = message_response_with_query(&full_router, "?limit=100").await;
        let tail_initial = message_response_with_query(&tail_router, "?limit=100").await;
        let mut full_page = full_initial.clone();
        for _ in 0..4 {
            let epoch = full_page.2["epoch"].as_str().unwrap();
            let cursor = full_page.2["olderCursor"].as_str().unwrap();
            let query = format!("?limit=100&epoch={epoch}&olderCursor={cursor}");
            full_page = message_response_with_query(&full_router, &query).await;
        }
        let epoch = full_page.2["epoch"].as_str().unwrap();
        let cursor = full_page.2["olderCursor"].as_str().unwrap();
        let stale_query = format!("?limit=100&epoch={epoch}&olderCursor={cursor}");
        let expected_page = message_response_with_query(&full_router, &stale_query).await;
        let actual_page = message_response_with_query(&tail_router, &stale_query).await;
        let cached_limit = tail_cache
            .transcript_parser_slot("fixture-session")
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|entry| entry.tail_window.as_ref())
            .map(|state| state.message_limit);

        assert_eq!(
            (tail_initial, actual_page, cached_limit),
            (full_initial, expected_page, Some(800)),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn older_epoch_pages_fall_back_at_the_transcript_start(
        #[with(150)] older_page_fixture: OlderPageFixture,
    ) {
        let OlderPageFixture {
            full_router,
            tail_router,
            tail_cache,
            ..
        } = older_page_fixture;
        let full_initial = message_response_with_query(&full_router, "?limit=100").await;
        let tail_initial = message_response_with_query(&tail_router, "?limit=100").await;
        let epoch = tail_initial.2["epoch"].as_str().unwrap();
        let cursor = tail_initial.2["olderCursor"].as_str().unwrap();
        let older_query = format!("?limit=100&epoch={epoch}&olderCursor={cursor}");
        let full_older = message_response_with_query(&full_router, &older_query).await;
        let tail_older = message_response_with_query(&tail_router, &older_query).await;
        let parser_uses_full_history = tail_cache
            .transcript_parser_slot("fixture-session")
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|entry| entry.tail_window.is_none());

        assert_eq!(
            (
                tail_initial,
                tail_older.2["olderCursor"].clone(),
                tail_older,
                parser_uses_full_history,
            ),
            (full_initial, serde_json::Value::Null, full_older, true,),
        );
    }

    #[tokio::test]
    async fn initial_tail_window_falls_back_when_an_unterminated_record_switches_branches() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let compressed_path = directory.path().join("transcript.jsonl.zst");
        let mut records = vec![
            json!({
                "type": "user",
                "uuid": "fixture-root",
                "parentUuid": null,
                "timestamp": "2026-01-01T00:00:00Z",
                "message": {"content": "Fixture root."},
            }),
            json!({
                "type": "system",
                "padding": "x".repeat(5 * 1024 * 1024),
            }),
        ];
        let mut parent_uuid = "fixture-root".to_owned();
        for index in 0..120 {
            let uuid = format!("fixture-old-branch-{index}");
            records.push(json!({
                "type": "user",
                "uuid": uuid,
                "parentUuid": parent_uuid,
                "timestamp": format!("2026-01-01T00:00:{:02}Z", index % 60),
                "message": {"content": format!("Abandoned fixture message {index}.")},
            }));
            parent_uuid = uuid;
        }
        let completed_records = records
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let pending_branch_switch = json!({
            "type": "user",
            "uuid": "fixture-current-branch",
            "parentUuid": "fixture-root",
            "timestamp": "2026-01-01T00:01:00Z",
            "message": {"content": "Current fixture branch."},
        })
        .to_string();
        let transcript = format!("{completed_records}\n{pending_branch_switch}");
        std::fs::write(&transcript_path, &transcript).unwrap();
        std::fs::write(
            &compressed_path,
            zstd::stream::encode_all(transcript.as_bytes(), 0).unwrap(),
        )
        .unwrap();

        let make_router = |path: &Path| {
            let mut session = stored_session();
            session.transcript_path = Some(path.to_path_buf());
            let cache = MessageCache::default();
            let mut loaders = empty_route_loaders();
            loaders.message_cache = cache.clone();
            let router = routes_with_proposals_and_peer_out_messages(
                move |_| Ok(Some(session.clone())),
                |_| Ok(None),
                loaders,
            );
            (router, cache)
        };
        let (full_router, _) = make_router(&compressed_path);
        let (tail_router, tail_cache) = make_router(&transcript_path);
        let full_response = message_response_with_query(&full_router, "?limit=100").await;
        let tail_response = message_response_with_query(&tail_router, "?limit=100").await;
        let tail_window_cached = tail_cache
            .transcript_parser_slot("fixture-session")
            .lock()
            .unwrap()
            .as_ref()
            .map(|entry| entry.tail_window.is_some());

        assert_eq!(
            (tail_response, tail_window_cached),
            (full_response, Some(false)),
        );
    }

    #[tokio::test]
    async fn epoch_refresh_reuses_tail_window_when_stored_files_enrich_anchor_messages() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let compressed_path = directory.path().join("transcript.jsonl.zst");
        let mut records = vec![json!({
            "type": "system",
            "padding": "x".repeat(5 * 1024 * 1024),
        })];
        let mut parent_uuid = None;
        for index in 0..120 {
            let uuid = format!("fixture-message-{index}");
            let timestamp_index = index + 1;
            records.push(json!({
                "type": "user",
                "uuid": uuid,
                "parentUuid": parent_uuid,
                "timestamp": format!(
                    "2026-01-01T00:{:02}:{:02}Z",
                    timestamp_index / 60,
                    timestamp_index % 60,
                ),
                "message": {"content": format!("Fixture request {index}.")},
            }));
            parent_uuid = Some(uuid);
        }
        let transcript = records
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&transcript_path, &transcript).unwrap();
        std::fs::write(
            &compressed_path,
            zstd::stream::encode_all(transcript.as_bytes(), 0).unwrap(),
        )
        .unwrap();

        let file_created_at = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:26.500Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let file = AgentFile {
            id: "fixture-stored-file".to_owned(),
            position: 0,
            name: "fixture-note.txt".to_owned(),
            caption: Some("Fixture attachment.".to_owned()),
            mime_type: Some("text/plain".to_owned()),
            size: 5,
            created_at: file_created_at,
        };
        let make_router = |path: &Path| {
            let route_file = file.clone();
            let mut session = stored_session();
            session.transcript_path = Some(path.to_path_buf());
            let cache = MessageCache::default();
            let loaders =
                empty_route_loaders().with_agent_file_loader(move |_| Ok(vec![route_file.clone()]));
            let mut loaders = loaders;
            loaders.message_cache = cache.clone();
            let router = routes_with_proposals_and_peer_out_messages(
                move |_| Ok(Some(session.clone())),
                |_| Ok(None),
                loaders,
            );
            (router, cache)
        };
        let (full_router, _) = make_router(&compressed_path);
        let (tail_router, tail_cache) = make_router(&transcript_path);
        let full_initial = message_response_with_query(&full_router, "?limit=100").await;
        let tail_initial = message_response_with_query(&tail_router, "?limit=100").await;
        let epoch = tail_initial.2["epoch"].as_str().unwrap();
        let older_cursor = tail_initial.2["olderCursor"].as_str().unwrap();
        let mut refresh_query = format!("?from=0&epoch={epoch}&olderCursor={older_cursor}");
        if let (Some(stable_start), Some(stable_end), Some(stable_digest)) = (
            tail_initial.2["stableStart"].as_u64(),
            tail_initial.2["stableEnd"].as_u64(),
            tail_initial.2["stableDigest"].as_str(),
        ) {
            refresh_query.push_str(&format!(
                "&stableStart={stable_start}&stableEnd={stable_end}&stableDigest={stable_digest}"
            ));
        }
        let full_refresh = message_response_with_query(&full_router, &refresh_query).await;
        let tail_refresh = message_response_with_query(&tail_router, &refresh_query).await;
        let tail_window_cached = tail_cache
            .transcript_parser_slot("fixture-session")
            .lock()
            .unwrap()
            .as_ref()
            .map(|entry| entry.tail_window.is_some());

        assert_eq!(
            (tail_initial, tail_refresh, tail_window_cached),
            (full_initial, full_refresh, Some(true)),
        );
    }

    #[tokio::test]
    async fn tail_window_parser_restarts_after_a_branch_switch() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let compressed_path = directory.path().join("full-transcript.jsonl.zst");
        let padding_record = json!({
            "type": "system",
            "padding": "x".repeat(5 * 1024 * 1024),
        })
        .to_string();
        let mut records = vec![padding_record];
        let mut parent_uuid = None;
        for index in 0..120 {
            let uuid = format!("fixture-message-{index}");
            records.push(
                json!({
                    "type": "user",
                    "uuid": uuid,
                    "parentUuid": parent_uuid,
                    "timestamp": "2026-01-01T00:00:01Z",
                    "message": {"content": format!("Fixture request {index}.")},
                })
                .to_string(),
            );
            parent_uuid = Some(uuid);
        }
        let transcript = format!("{}\n", records.join("\n"));
        std::fs::write(&transcript_path, &transcript).unwrap();
        std::fs::write(
            &compressed_path,
            zstd::stream::encode_all(transcript.as_bytes(), 0).unwrap(),
        )
        .unwrap();

        let make_router = |path: &Path| {
            let mut session = stored_session();
            session.transcript_path = Some(path.to_path_buf());
            let cache = MessageCache::default();
            let mut loaders = empty_route_loaders();
            loaders.message_cache = cache.clone();
            (
                routes_with_proposals_and_peer_out_messages(
                    move |_| Ok(Some(session.clone())),
                    |_| Ok(None),
                    loaders,
                ),
                cache,
            )
        };
        let (full_router, _) = make_router(&compressed_path);
        let (tail_router, tail_cache) = make_router(&transcript_path);
        let initial = message_response_with_query(&tail_router, "?limit=100").await;
        let epoch = initial.2["epoch"].as_str().unwrap();
        let parser_slot = tail_cache.transcript_parser_slot("fixture-session");
        assert_eq!(
            parser_slot
                .lock()
                .unwrap()
                .as_ref()
                .map(|entry| entry.tail_window.is_some()),
            Some(true),
        );
        let switched_branch = json!({
            "type": "user",
            "uuid": "fixture-switched-branch",
            "parentUuid": "fixture-parent-outside-window",
            "timestamp": "2026-01-01T00:00:02Z",
            "message": {"content": "The fixture switched branches."},
        })
        .to_string();
        let switched_transcript = format!("{transcript}{switched_branch}\n");
        std::fs::write(&transcript_path, &switched_transcript).unwrap();
        std::fs::write(
            &compressed_path,
            zstd::stream::encode_all(switched_transcript.as_bytes(), 0).unwrap(),
        )
        .unwrap();

        let query = format!("?from=0&epoch={epoch}");
        let full_response = message_response_with_query(&full_router, &query).await;
        let tail_response = message_response_with_query(&tail_router, &query).await;
        let tail_window_cached = parser_slot
            .lock()
            .unwrap()
            .as_ref()
            .map(|entry| entry.tail_window.is_some());

        assert_eq!(
            (tail_response, tail_window_cached),
            (full_response, Some(false)),
        );
    }

    #[tokio::test]
    async fn tail_window_falls_back_when_the_requested_epoch_is_outside_the_window() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let compressed_path = directory.path().join("full-transcript.jsonl.zst");
        let mut records = Vec::new();
        let mut parent_uuid = None;
        for index in 0..120 {
            let uuid = format!("fixture-old-message-{index}");
            records.push(
                json!({
                    "type": "user",
                    "uuid": uuid,
                    "parentUuid": parent_uuid,
                    "timestamp": "2026-01-01T00:00:01Z",
                    "message": {"content": format!("Earlier fixture request {index}.")},
                })
                .to_string(),
            );
            parent_uuid = Some(uuid);
        }
        records.push(json!({"fixturePadding": "x".repeat(5 * 1024 * 1024)}).to_string());
        for index in 0..120 {
            let uuid = format!("fixture-current-message-{index}");
            records.push(
                json!({
                    "type": "user",
                    "uuid": uuid,
                    "parentUuid": parent_uuid,
                    "timestamp": "2026-01-01T00:00:02Z",
                    "message": {"content": format!("Current fixture request {index}.")},
                })
                .to_string(),
            );
            parent_uuid = Some(uuid);
        }
        let transcript = format!("{}\n", records.join("\n"));
        std::fs::write(&transcript_path, &transcript).unwrap();
        std::fs::write(
            &compressed_path,
            zstd::stream::encode_all(transcript.as_bytes(), 0).unwrap(),
        )
        .unwrap();

        let make_router = |path: &Path| {
            let mut session = stored_session();
            session.transcript_path = Some(path.to_path_buf());
            let cache = MessageCache::default();
            let mut loaders = empty_route_loaders();
            loaders.message_cache = cache.clone();
            (
                routes_with_proposals_and_peer_out_messages(
                    move |_| Ok(Some(session.clone())),
                    |_| Ok(None),
                    loaders,
                ),
                cache,
            )
        };
        let (full_router, _) = make_router(&compressed_path);
        let (tail_router, tail_cache) = make_router(&transcript_path);
        let full_initial = message_response_with_query(&full_router, "?limit=100").await;
        let full_epoch = full_initial.2["epoch"].as_str().unwrap();
        let full_older_cursor = full_initial.2["olderCursor"].as_str().unwrap();
        let full_older_page = message_response_with_query(
            &full_router,
            &format!("?limit=100&epoch={full_epoch}&olderCursor={full_older_cursor}"),
        )
        .await;
        let older_epoch = full_older_page.2["epoch"].as_str().unwrap();
        let query = format!("?from=0&epoch={older_epoch}");
        let full_response = message_response_with_query(&full_router, &query).await;
        let tail_initial = message_response_with_query(&tail_router, "?limit=100").await;
        let parser_slot = tail_cache.transcript_parser_slot("fixture-session");
        let tail_window_cached_before_request = parser_slot
            .lock()
            .unwrap()
            .as_ref()
            .map(|entry| entry.tail_window.is_some());
        let tail_response = message_response_with_query(&tail_router, &query).await;
        let tail_window_cached_after_request = parser_slot
            .lock()
            .unwrap()
            .as_ref()
            .map(|entry| entry.tail_window.is_some());

        assert_eq!(
            (
                tail_initial,
                tail_response,
                tail_window_cached_before_request,
                tail_window_cached_after_request,
            ),
            (full_initial, full_response, Some(true), Some(false),),
        );
    }

    #[tokio::test]
    async fn epoch_request_resets_when_its_anchor_is_missing() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let mut records = Vec::new();
        let mut parent_uuid = None;
        for index in 0..120 {
            let uuid = format!("fixture-message-{index}");
            records.push(json!({
                "type": "user",
                "uuid": uuid,
                "parentUuid": parent_uuid,
                "timestamp": "2026-01-01T00:00:01Z",
                "message": {"content": format!("Old fixture message {index}.")},
            }));
            parent_uuid = Some(uuid);
        }
        let transcript = records
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&transcript_path, transcript).unwrap();

        let mut session = stored_session();
        session.transcript_path = Some(transcript_path.clone());
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );
        let initial_response = message_response_with_query(&router, "?limit=100").await;
        let stale_epoch = initial_response.2["epoch"].as_str().unwrap().to_owned();

        let replacement_record = json!({
            "type": "user",
            "uuid": "fixture-replacement-root",
            "parentUuid": null,
            "timestamp": "2026-01-02T00:00:01Z",
            "message": {"content": "Replacement fixture message."},
        });
        std::fs::write(&transcript_path, replacement_record.to_string()).unwrap();
        let stale_response = message_response_with_query(
            &router,
            &format!("?from=0&epoch={stale_epoch}&olderCursor={stale_epoch}"),
        )
        .await;
        let mut expected_response = message_response_with_query(&router, "?limit=100").await;
        expected_response.2["epochReset"] = json!(true);

        assert_eq!(stale_response, expected_response);
    }

    #[tokio::test]
    async fn codex_message_route_loads_and_resolves_saved_prompts_by_id() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            r#"{"type":"event_msg","timestamp":"2026-01-01T00:00:01Z","payload":{"type":"item_completed","item":{"type":"UserMessage","content":"Engine-formatted Codex delivery.\n<!-- armyknife-queued-prompt-id:fixture-message-8 -->"}}}"#,
        )
        .unwrap();
        let mut session = stored_session();
        session.engine = Engine::Codex;
        session.transcript_path = Some(transcript_path);
        let queued = saved_prompt(8, "Codex delivered request.");
        let marked_ids = Arc::new(Mutex::new(Vec::new()));
        let mark_log = Arc::clone(&marked_ids);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            move |_| Ok(vec![queued.clone()]),
            |_| Ok(Vec::new()),
            move |_, ids| {
                mark_log.lock().unwrap().extend_from_slice(ids);
                Ok(())
            },
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                response_snapshot(response).await,
                marked_ids.lock().unwrap().clone(),
            ),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    r#"{"messages":[{"type":"user","text":"Codex delivered request.","queuedPromptId":"fixture-message-8","timestamp":"2026-01-01T00:00:01+00:00"}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null}"#.to_string(),
                ),
                vec![8],
            ),
        );
    }

    #[tokio::test]
    async fn codex_message_route_includes_registered_background_run_tasks() {
        let mut session = stored_session();
        session.engine = Engine::Codex;
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_background_task_loader(|session_id| {
            Ok((session_id == "fixture-session")
                .then(|| super::BackgroundTask {
                    id: "fixture-background-task-id".to_owned(),
                    description: "Background command".to_owned(),
                    elapsed_seconds: 5,
                    kind: super::BackgroundTaskKind::BackgroundTask,
                    output_path: None,
                    agent_type: None,
                })
                .into_iter()
                .collect())
        });
        let router = routes_with_proposals_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            loaders,
        );

        assert_eq!(
            message_response(&router).await.2,
            json!({
                "messages": [],
                "backgroundTasks": [{
                    "id": "fixture-background-task-id",
                    "description": "Background command",
                    "elapsedSeconds": 5,
                    "kind": "background_task",
                }],
                "start": 0,
                "total": 0,
                "decisions": [],
                "hasUnansweredProposal": false,
                "activeQuestionIndex": null,
            }),
        );
    }

    #[tokio::test]
    async fn claude_message_route_returns_running_background_tasks() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let task_started_at = chrono::Utc::now() - chrono::Duration::seconds(100);
        let records = [
            json!({
                "uuid": "fixture-prompt-row",
                "parentUuid": null,
                "type": "user",
                "message": {"content": "Watch the fixture build."},
            }),
            json!({
                "uuid": "fixture-bash-call-row",
                "parentUuid": "fixture-prompt-row",
                "type": "assistant",
                "timestamp": task_started_at.to_rfc3339(),
                "message": {"content": [{
                    "type": "tool_use",
                    "id": "fixture-bash-tool-use-id",
                    "name": "Bash",
                    "input": {
                        "command": "watch fixture build",
                        "description": "Watch the synthetic build",
                        "run_in_background": true,
                    },
                }]},
            }),
            json!({
                "uuid": "fixture-bash-result-row",
                "parentUuid": "fixture-bash-call-row",
                "type": "user",
                "sourceToolAssistantUUID": "fixture-bash-call-row",
                "toolUseResult": {},
                "message": {"content": [{
                    "type": "tool_result",
                    "tool_use_id": "fixture-bash-tool-use-id",
                    "content": "Command running in background with ID: fixture-task-id. Output is being written to: /fixture/task-output.log",
                }]},
            }),
        ];
        std::fs::write(
            &transcript_path,
            records
                .iter()
                .map(serde_json::to_string)
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .join("\n"),
        )
        .unwrap();

        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );
        let first = message_response(&router).await.2;
        let mut normalized_first = first.clone();
        normalized_first["backgroundTasks"][0]["elapsedSeconds"] = json!(0);
        assert_eq!(
            normalized_first,
            json!({
                "messages": [
                    {
                        "type": "user",
                        "text": "Watch the fixture build.",
                        "rewindId": "fixture-prompt-row",
                    },
                    {
                        "type": "tool_summary",
                        "summary": "Bash ×1",
                        "operations": [{"tool": "Bash", "target": "watch fixture build", "failed": false}],
                    },
                ],
                "backgroundTasks": [{
                    "id": "fixture-task-id",
                    "description": "Watch the synthetic build",
                    "elapsedSeconds": 0,
                    "kind": "background_task",
                }],
                "start": 0,
                "total": 2,
                "decisions": [],
                "hasUnansweredProposal": false,
                "activeQuestionIndex": null,
            }),
        );

        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
        let second = message_response(&router).await.2;
        let mut expected_second = first.clone();
        expected_second["backgroundTasks"][0]["elapsedSeconds"] = json!(
            first["backgroundTasks"][0]["elapsedSeconds"]
                .as_u64()
                .unwrap()
                + 1
        );

        assert_eq!(second, expected_second);
    }

    #[tokio::test]
    async fn background_task_output_route_reads_only_the_task_output_in_its_transcript() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let output_path = directory.path().join("task-output.log");
        std::fs::write(&output_path, "fixture output\n").unwrap();
        let router = background_task_output_router(&transcript_path, &output_path);
        let output_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/background-tasks/fixture-task-id/output")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let output_status = output_response.status();
        let output_cache_policy = output_response
            .headers()
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let output = to_bytes(output_response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        std::fs::write(&output_path, "updated fixture output\n").unwrap();
        let updated_output_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/background-tasks/fixture-task-id/output")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let updated_output = to_bytes(updated_output_response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        let unknown_task_response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/background-tasks/other-task/output")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unknown_task_status = unknown_task_response.status();
        let unknown_task = to_bytes(unknown_task_response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();

        assert_eq!(
            (
                output_status,
                output_cache_policy,
                output,
                updated_output,
                unknown_task_status,
                unknown_task,
            ),
            (
                StatusCode::OK,
                Some("no-store".to_string()),
                b"fixture output\n".to_vec(),
                b"updated fixture output\n".to_vec(),
                StatusCode::OK,
                b"Output is unavailable for this task.".to_vec(),
            ),
        );
    }

    #[tokio::test]
    async fn codex_background_task_output_route_uses_registered_output_path() {
        let directory = tempfile::tempdir().unwrap();
        let output_path = directory.path().join("registered-output.log");
        std::fs::write(&output_path, "registered fixture output\n").unwrap();

        let mut session = stored_session();
        session.engine = Engine::Codex;
        session.transcript_path = None;
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_background_task_loader({
            let output_path = output_path.clone();
            move |session_id| {
                Ok((session_id == "fixture-session")
                    .then(|| super::BackgroundTask {
                        id: "fixture-background-task-id".to_owned(),
                        description: "Background command".to_owned(),
                        elapsed_seconds: 1,
                        kind: super::BackgroundTaskKind::BackgroundTask,
                        output_path: Some(output_path.clone()),
                        agent_type: None,
                    })
                    .into_iter()
                    .collect())
            }
        });
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            loaders,
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri(
                        "/sessions/fixture-session/background-tasks/fixture-background-task-id/output",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();

        assert_eq!(
            (status, body),
            (StatusCode::OK, b"registered fixture output\n".to_vec(),),
        );
    }

    #[tokio::test]
    async fn background_task_output_events_route_streams_updates_from_injected_events() {
        let directory = tempfile::tempdir().unwrap();
        let output_path = directory.path().join("registered-output.log");
        std::fs::write(&output_path, "first output").unwrap();

        let mut session = stored_session();
        session.engine = Engine::Codex;
        session.transcript_path = None;
        let loaders = RouteLoaders::new_with_background_task_output_event_stream(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
            |path| {
                Ok(Box::pin(stream::iter([Ok(notify::Event::new(
                    notify::EventKind::Any,
                )
                .add_path(path))])))
            },
        )
        .with_background_task_loader({
            let output_path = output_path.clone();
            move |session_id| {
                Ok((session_id == "fixture-session")
                    .then(|| super::BackgroundTask {
                        id: "fixture-background-task-id".to_owned(),
                        description: "Background command".to_owned(),
                        elapsed_seconds: 1,
                        kind: super::BackgroundTaskKind::BackgroundTask,
                        output_path: Some(output_path.clone()),
                        agent_type: None,
                    })
                    .into_iter()
                    .collect())
            }
        });
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            loaders,
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri(
                        "/sessions/fixture-session/background-tasks/fixture-background-task-id/output/events",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let cache_control = response
            .headers()
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_options = response
            .headers()
            .get(axum::http::header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut body = response.into_body().into_data_stream();
        let first_event = body.next().await.unwrap().unwrap().to_vec();
        std::fs::write(&output_path, "second output").unwrap();
        let second_event = body.next().await.unwrap().unwrap().to_vec();

        assert_eq!(
            (
                status,
                content_type,
                cache_control,
                content_options,
                first_event,
                second_event,
            ),
            (
                StatusCode::OK,
                Some("text/event-stream".to_owned()),
                Some("no-store".to_owned()),
                Some("nosniff".to_owned()),
                indoc::indoc! {"
                    data: first output

                "}
                .as_bytes()
                .to_vec(),
                indoc::indoc! {"
                    data: second output

                "}
                .as_bytes()
                .to_vec(),
            ),
        );
    }

    #[tokio::test]
    async fn background_task_output_events_route_reports_unavailable_output() {
        let mut session = stored_session();
        session.engine = Engine::Codex;
        session.transcript_path = None;
        let loaders = empty_route_loaders().with_background_task_loader(|session_id| {
            Ok((session_id == "fixture-session")
                .then(|| super::BackgroundTask {
                    id: "fixture-background-task-id".to_owned(),
                    description: "Background command".to_owned(),
                    elapsed_seconds: 1,
                    kind: super::BackgroundTaskKind::BackgroundTask,
                    output_path: None,
                    agent_type: None,
                })
                .into_iter()
                .collect())
        });
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            loaders,
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri(
                        "/sessions/fixture-session/background-tasks/fixture-background-task-id/output/events",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let cache_control = response
            .headers()
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_options = response
            .headers()
            .get(axum::http::header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut body = response.into_body().into_data_stream();
        let event = body.next().await.unwrap().unwrap().to_vec();
        let stream_state =
            match tokio::time::timeout(std::time::Duration::from_millis(100), body.next()).await {
                Err(_) => "pending",
                Ok(None) => "closed",
                Ok(Some(Err(_))) => "error",
                Ok(Some(Ok(_))) => "extra event",
            };

        assert_eq!(
            (
                status,
                content_type,
                cache_control,
                content_options,
                event,
                stream_state,
            ),
            (
                StatusCode::OK,
                Some("text/event-stream".to_owned()),
                Some("no-store".to_owned()),
                Some("nosniff".to_owned()),
                indoc::indoc! {"
                    data: Output is unavailable for this task.

                "}
                .as_bytes()
                .to_vec(),
                "pending",
            ),
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn background_task_output_route_rejects_symlink_targets() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let output_path = directory.path().join("task-output.log");
        let target_path = directory.path().join("unrelated.log");
        std::fs::write(&target_path, "synthetic secret\n").unwrap();
        std::os::unix::fs::symlink(&target_path, &output_path).unwrap();
        let router = background_task_output_router(&transcript_path, &output_path);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/background-tasks/fixture-task-id/output")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();

        assert_eq!(
            (status, body),
            (
                StatusCode::OK,
                b"Output is unavailable for this task.".to_vec(),
            ),
        );
    }

    #[tokio::test]
    async fn active_tool_route_returns_the_frontend_tool_item_shape() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        let record = json!({
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call",
                "call_id": "fixture-call-id",
                "name": "exec",
                "input": "cargo check",
            },
        });
        std::fs::write(&transcript_path, serde_json::to_vec(&record).unwrap()).unwrap();

        let mut session = stored_session();
        session.engine = Engine::Codex;
        session.transcript_path = Some(transcript_path);
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response_snapshot(response).await,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                r#"{"messages":[{"type":"tool_in_progress","tool":"exec","target":"cargo check"}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null}"#.to_string(),
            )
        );
    }

    #[rstest]
    #[case::explicit(true, false, false, true, "enabled", None)]
    #[case::inherited(false, true, true, true, "inherited", Some("fixture-parent"))]
    #[case::disabled(false, false, true, true, "disabled", None)]
    #[case::repository_disabled(true, false, false, false, "repo_disabled", None)]
    #[tokio::test]
    async fn messages_route_includes_vibe_state(
        #[case] explicit: bool,
        #[case] launched_with_vibe: bool,
        #[case] has_parent: bool,
        #[case] repo_vibe_allowed: bool,
        #[case] expected_mode: &str,
        #[case] expected_ancestor: Option<&str>,
    ) {
        let mut session = stored_session();
        session.vibe_enabled = explicit;
        session.launched_with_vibe = launched_with_vibe;
        let parent = if has_parent {
            session.ancestor_session_ids = vec!["fixture-parent".to_string()];
            let mut parent = stored_session();
            parent.session_id = "fixture-parent".to_string();
            parent.vibe_enabled = true;
            Some(parent)
        } else {
            None
        };
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_vibe_context(
            move || {
                if !repo_vibe_allowed {
                    return Config::default();
                }
                serde_yaml::from_str::<Config>(indoc::indoc! {"
                    orgs:
                      example-org:
                        vibe: true
                "})
                .unwrap()
            },
            |_| Some(("example-org".to_string(), "example-repo".to_string())),
        );
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| match id {
                "fixture-session" => Ok(Some(session.clone())),
                "fixture-parent" => Ok(parent.clone()),
                _ => Ok(None),
            },
            |_| Ok(None),
            loaders,
        );
        let mut vibe_mode = json!({ "mode": expected_mode });
        if let Some(ancestor_session_id) = expected_ancestor {
            vibe_mode["ancestorSessionId"] = json!(ancestor_session_id);
        }

        assert_eq!(
            message_response(&router).await,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                json!({
                    "messages": [],
                    "backgroundTasks": [],
                    "start": 0,
                    "total": 0,
                    "decisions": [],
                    "hasUnansweredProposal": false,
                    "activeQuestionIndex": null,
                    "vibeMode": vibe_mode,
                }),
            ),
        );
    }

    #[tokio::test]
    async fn invalid_older_ancestor_does_not_break_messages_or_vibe_state() {
        let mut session = stored_session();
        session.launched_with_vibe = true;
        session.ancestor_session_ids = vec![
            "../stale-ancestor".to_string(),
            "fixture-parent".to_string(),
        ];
        let mut parent = stored_session();
        parent.session_id = "fixture-parent".to_string();
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_vibe_context(
            || {
                serde_yaml::from_str::<Config>(indoc::indoc! {"
                    orgs:
                      example-org:
                        vibe: true
                "})
                .unwrap()
            },
            |_| Some(("example-org".to_string(), "example-repo".to_string())),
        );
        let router = routes_with_proposals_and_peer_out_messages(
            move |id| match id {
                "fixture-session" => Ok(Some(session.clone())),
                "fixture-parent" => Ok(Some(parent.clone())),
                "../stale-ancestor" => Err(
                    crate::commands::agent::error::CcError::InvalidSessionId(id.to_string()).into(),
                ),
                _ => Ok(None),
            },
            |_| Ok(None),
            loaders,
        );

        assert_eq!(
            message_response(&router).await,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                json!({
                    "messages": [],
                    "backgroundTasks": [],
                    "start": 0,
                    "total": 0,
                    "decisions": [],
                    "hasUnansweredProposal": false,
                    "activeQuestionIndex": null,
                    "vibeMode": { "mode": "disabled" },
                }),
            ),
        );
    }
}
