use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::Body;
use axum::extract::Path as AxumPath;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;
use serde_json::Value;

use crate::commands::agent::claude_sessions;
use crate::commands::agent::types::{Engine, MAIN_THREAD_AGENT_KEY, Session};
use crate::commands::agent::user_turn::is_ask_user_question;

mod parser;

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

type SessionLoader = Arc<dyn Fn(&str) -> Result<Option<Session>> + Send + Sync>;
type UserMarkerLoader = Arc<dyn Fn(&str) -> Result<Option<String>> + Send + Sync>;

pub(super) fn routes_with(
    load_session: impl Fn(&str) -> Result<Option<Session>> + Send + Sync + 'static,
    load_user_marker: impl Fn(&str) -> Result<Option<String>> + Send + Sync + 'static,
) -> Router {
    Router::new()
        .route("/sessions/{id}/messages", get(get_messages))
        .with_state((
            Arc::new(load_session) as SessionLoader,
            Arc::new(load_user_marker) as UserMarkerLoader,
        ))
}

async fn get_messages(
    State((load_session, load_user_marker)): State<(SessionLoader, UserMarkerLoader)>,
    AxumPath(session_id): AxumPath<String>,
) -> Response {
    match tokio::task::spawn_blocking(move || {
        load_messages_with_marker(
            &session_id,
            |id| load_session(id),
            |id| load_user_marker(id),
        )
    })
    .await
    {
        Ok(Ok(Some(messages))) => match serde_json::to_vec(&messages) {
            Ok(body) => (
                StatusCode::OK,
                [(CONTENT_TYPE, "application/json")],
                Body::from(body),
            )
                .into_response(),
            Err(error) => {
                tracing::warn!(%error, "failed to serialize agent session messages");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        },
        Ok(Ok(None)) => StatusCode::NOT_FOUND.into_response(),
        Ok(Err(error)) => {
            tracing::warn!(%error, "failed to read agent session messages");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        Err(error) => {
            tracing::error!(%error, "agent session message worker failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
fn load_messages_with(
    session_id: &str,
    load_session: impl FnMut(&str) -> Result<Option<Session>>,
) -> Result<Option<Vec<ChatItem>>> {
    load_messages_with_marker(session_id, load_session, |_| Ok(None))
}

fn load_messages_with_marker(
    session_id: &str,
    mut load_session: impl FnMut(&str) -> Result<Option<Session>>,
    load_user_marker: impl FnOnce(&str) -> Result<Option<String>>,
) -> Result<Option<Vec<ChatItem>>> {
    let Some(session) = load_session(session_id)? else {
        return Ok(None);
    };
    let user_marker = if session.engine == Engine::Claude {
        load_user_marker(session_id)?
    } else {
        None
    };
    let parent = parent_session(&session, &mut load_session)?;
    let main_permission_pending = session
        .pending_permission_agent_ids
        .contains(MAIN_THREAD_AGENT_KEY);
    let main_permission_is_question = is_ask_user_question(&session);
    let pending_tool = (main_permission_pending && !main_permission_is_question)
        .then_some(session.current_tool.as_deref())
        .flatten();
    let mut messages = session
        .transcript_path
        .as_deref()
        .map(|path| {
            parse_session_transcript(
                path,
                session.engine,
                parent.as_ref(),
                pending_tool,
                user_marker.as_deref(),
            )
        })
        .transpose()?
        .unwrap_or_default();

    if main_permission_pending
        && !main_permission_is_question
        && !messages
            .iter()
            .any(|message| matches!(message, ChatItem::Permission { .. }))
    {
        messages.push(ChatItem::Permission {
            tool: session
                .current_tool
                .clone()
                .unwrap_or_else(|| "Permission required".to_string()),
            description: session
                .last_message
                .clone()
                .unwrap_or_else(|| "Waiting for permission".to_string()),
        });
    }

    for agent_id in session
        .pending_permission_agent_ids
        .iter()
        .filter(|agent_id| agent_id.as_str() != MAIN_THREAD_AGENT_KEY)
    {
        messages.push(ChatItem::Permission {
            tool: format!("Subagent {agent_id}"),
            description: "Waiting for permission approval.".to_string(),
        });
    }

    Ok(Some(messages))
}

fn parent_session(
    session: &Session,
    load_session: &mut impl FnMut(&str) -> Result<Option<Session>>,
) -> Result<Option<ParentSession>> {
    let Some(parent_id) = session.ancestor_session_ids.last() else {
        return Ok(None);
    };
    let Some(parent) = load_session(parent_id)? else {
        return Ok(None);
    };
    let title = parent
        .label
        .or_else(|| claude_sessions::get_session_title(&parent.cwd, &parent.session_id))
        .or_else(|| {
            parent
                .cwd
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "parent session".to_string());
    Ok(Some(ParentSession {
        session_id: parent.session_id,
        title,
    }))
}

fn parse_session_transcript(
    path: &Path,
    engine: Engine,
    parent: Option<&ParentSession>,
    pending_tool: Option<&str>,
    watch_web_user_marker: Option<&str>,
) -> Result<Vec<ChatItem>> {
    match engine {
        Engine::Claude => {
            let Some(reader) = open_seekable_transcript(path)? else {
                return Ok(Vec::new());
            };
            Ok(parser::claude::parse(
                reader,
                parent,
                pending_tool,
                watch_web_user_marker,
            )?)
        }
        Engine::Codex => {
            let Some(reader) = open_transcript(path, true)? else {
                return Ok(Vec::new());
            };
            Ok(parser::codex::parse(reader, parent)?)
        }
    }
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
        return Ok(None);
    }
    let compressed_path = compressed_path(path);
    let file = match File::open(&compressed_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("open transcript {}", compressed_path.display()));
        }
    };
    let decoder = zstd::stream::read::Decoder::new(file)
        .with_context(|| format!("decompress transcript {}", compressed_path.display()))?;
    Ok(Some(Box::new(BufReader::new(decoder))))
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ChatItem {
    User {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    UserWithContent {
        content: Vec<UserContentPart>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    Assistant {
        text: String,
    },
    ToolSummary {
        summary: String,
        operations: Vec<ToolOperation>,
    },
    ToolInProgress {
        tool: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },
    Question {
        question: String,
        choices: Vec<String>,
        answer: Option<String>,
    },
    Permission {
        tool: String,
        description: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum UserContentPart {
    Text { text: String },
    Image { source: String, available: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ToolOperation {
    tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<String>,
}

struct ParentSession {
    session_id: String,
    title: String,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use axum::http::header::CONTENT_TYPE;
    use serde_json::json;
    use tower::ServiceExt;

    use super::{ChatItem, load_messages_with, routes_with};
    use crate::commands::agent::types::{Engine, MAIN_THREAD_AGENT_KEY, Session};

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
                            "options": [{"label": "Yes"}, {"label": "No"}],
                        }],
                    },
                }],
            },
        });
        std::fs::write(&transcript_path, serde_json::to_vec(&record).unwrap()).unwrap();
        (directory, transcript_path)
    }

    #[test]
    fn missing_store_entry_has_no_messages_result() {
        assert_eq!(
            load_messages_with("fixture-unregistered-session", |_| Ok(None)).unwrap(),
            None
        );
    }

    #[test]
    fn pending_permission_without_transcript_uses_the_stored_request_details() {
        let mut session = stored_session();
        session.pending_permission_agent_ids = BTreeSet::from([MAIN_THREAD_AGENT_KEY.to_string()]);
        session.current_tool = Some("Bash(cargo test -p sample)".to_string());
        session.last_message = Some("Run the package tests".to_string());

        assert_eq!(
            load_messages_with("fixture-session", |id| {
                Ok((id == "fixture-session").then(|| session.clone()))
            })
            .unwrap(),
            Some(vec![ChatItem::Permission {
                tool: "Bash(cargo test -p sample)".to_string(),
                description: "Run the package tests".to_string(),
            }])
        );
    }

    #[test]
    fn pending_question_does_not_create_a_permission_card() {
        let (_directory, transcript_path) = pending_question_transcript();

        let mut session = stored_session();
        session.pending_permission_agent_ids = BTreeSet::from([MAIN_THREAD_AGENT_KEY.to_string()]);
        session.current_tool = Some("AskUserQuestion".to_string());
        session.transcript_path = Some(transcript_path);

        assert_eq!(
            load_messages_with("fixture-session", |id| {
                Ok((id == "fixture-session").then(|| session.clone()))
            })
            .unwrap(),
            Some(vec![ChatItem::Question {
                question: "Should the summary be brief?".to_string(),
                choices: vec!["Yes".to_string(), "No".to_string()],
                answer: None,
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
        session.current_tool = Some("AskUserQuestion".to_string());
        session.last_message = Some("Should the summary be brief?".to_string());
        session.transcript_path = Some(transcript_path);

        assert_eq!(
            load_messages_with("fixture-session", |id| {
                Ok((id == "fixture-session").then(|| session.clone()))
            })
            .unwrap(),
            Some(vec![
                ChatItem::Question {
                    question: "Should the summary be brief?".to_string(),
                    choices: vec!["Yes".to_string(), "No".to_string()],
                    answer: None,
                },
                ChatItem::Permission {
                    tool: "Subagent fixture-agent-id".to_string(),
                    description: "Waiting for permission approval.".to_string(),
                },
            ])
        );
    }

    #[tokio::test]
    async fn peer_message_route_attributes_only_the_stored_parent() {
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
        child.ancestor_session_ids = vec!["fixture-parent-id".to_string()];
        child.transcript_path = Some(transcript_path);
        let mut parent = stored_session();
        parent.session_id = "fixture-parent-id".to_string();
        parent.label = Some("Fixture parent".to_string());

        let router = routes_with(
            move |id| {
                Ok(match id {
                    "fixture-child-id" => Some(child.clone()),
                    "fixture-parent-id" => Some(parent.clone()),
                    _ => None,
                })
            },
            |_| Ok(None),
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

        assert_eq!(
            (status, content_type, body),
            (
                axum::http::StatusCode::OK,
                Some("application/json".to_string()),
                r#"[{"type":"user","text":"Synthetic peer request.","from":"Fixture parent"}]"#
                    .to_string(),
            )
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
        let router = routes_with(move |_| Ok(Some(session.clone())), |_| Ok(None));
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
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

        assert_eq!(
            (status, content_type, body),
            (
                axum::http::StatusCode::OK,
                Some("application/json".to_string()),
                r#"[{"type":"tool_in_progress","tool":"exec","target":"cargo check"}]"#.to_string(),
            )
        );
    }
}
