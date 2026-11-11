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
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::agent::claude_sessions;
use crate::commands::agent::store::{AgentProposal, WatchWebQueuedPrompt};
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
type QueuedPromptLoader = Arc<dyn Fn(&str) -> Result<Vec<WatchWebQueuedPrompt>> + Send + Sync>;
type QueuedPromptDeleter = Arc<dyn Fn(&str, &[i64]) -> Result<()> + Send + Sync>;
type ProposalLoader = Arc<dyn Fn(&str) -> Result<Vec<AgentProposal>> + Send + Sync>;

#[cfg(test)]
pub(super) fn routes_with_queued_prompts(
    load_session: impl Fn(&str) -> Result<Option<Session>> + Send + Sync + 'static,
    load_user_marker: impl Fn(&str) -> Result<Option<String>> + Send + Sync + 'static,
    load_queued_prompts: impl Fn(&str) -> Result<Vec<WatchWebQueuedPrompt>> + Send + Sync + 'static,
    delete_queued_prompts: impl Fn(&str, &[i64]) -> Result<()> + Send + Sync + 'static,
) -> Router {
    routes_with_proposals(
        load_session,
        load_user_marker,
        load_queued_prompts,
        delete_queued_prompts,
        |_| Ok(Vec::new()),
    )
}

pub(super) fn routes_with_proposals(
    load_session: impl Fn(&str) -> Result<Option<Session>> + Send + Sync + 'static,
    load_user_marker: impl Fn(&str) -> Result<Option<String>> + Send + Sync + 'static,
    load_queued_prompts: impl Fn(&str) -> Result<Vec<WatchWebQueuedPrompt>> + Send + Sync + 'static,
    delete_queued_prompts: impl Fn(&str, &[i64]) -> Result<()> + Send + Sync + 'static,
    load_proposals: impl Fn(&str) -> Result<Vec<AgentProposal>> + Send + Sync + 'static,
) -> Router {
    Router::new()
        .route("/sessions/{id}/messages", get(get_messages))
        .with_state((
            Arc::new(load_session) as SessionLoader,
            Arc::new(load_user_marker) as UserMarkerLoader,
            Arc::new(load_queued_prompts) as QueuedPromptLoader,
            Arc::new(delete_queued_prompts) as QueuedPromptDeleter,
            Arc::new(load_proposals) as ProposalLoader,
        ))
}

async fn get_messages(
    State((
        load_session,
        load_user_marker,
        load_queued_prompts,
        delete_queued_prompts,
        load_proposals,
    )): State<(
        SessionLoader,
        UserMarkerLoader,
        QueuedPromptLoader,
        QueuedPromptDeleter,
        ProposalLoader,
    )>,
    AxumPath(session_id): AxumPath<String>,
) -> Response {
    match tokio::task::spawn_blocking(move || {
        load_conversation_with_queued_prompts(
            &session_id,
            |id| load_session(id),
            |id| load_user_marker(id),
            |id| load_queued_prompts(id),
            |id, ids| delete_queued_prompts(id, ids),
            |id| load_proposals(id),
        )
    })
    .await
    {
        Ok(Ok(Some(conversation))) => match serde_json::to_vec(&conversation) {
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
    Ok(load_conversation_with_queued_prompts(
        session_id,
        load_session,
        |_| Ok(None),
        |_| Ok(Vec::new()),
        |_, _| Ok(()),
        |_| Ok(Vec::new()),
    )?
    .map(|conversation| conversation.messages))
}

fn load_conversation_with_queued_prompts(
    session_id: &str,
    mut load_session: impl FnMut(&str) -> Result<Option<Session>>,
    load_user_marker: impl FnOnce(&str) -> Result<Option<String>>,
    load_queued_prompts: impl FnOnce(&str) -> Result<Vec<WatchWebQueuedPrompt>>,
    delete_queued_prompts: impl FnOnce(&str, &[i64]) -> Result<()>,
    load_proposals: impl FnOnce(&str) -> Result<Vec<AgentProposal>>,
) -> Result<Option<ConversationResponse>> {
    let Some(session) = load_session(session_id)? else {
        return Ok(None);
    };
    let user_marker = if session.engine == Engine::Claude {
        load_user_marker(session_id)?
    } else {
        None
    };
    let queued_prompts = if session.engine == Engine::Claude {
        load_queued_prompts(session_id)?
    } else {
        Vec::new()
    };
    let proposals = load_proposals(session_id)?;
    let proposal_items = proposals
        .iter()
        .map(proposal_chat_item)
        .collect::<Result<Vec<_>>>()?;
    let parent = parent_session(&session, &mut load_session)?;
    let main_permission_pending = session
        .pending_permission_agent_ids
        .contains(MAIN_THREAD_AGENT_KEY);
    let main_permission_is_question = is_ask_user_question(&session);
    let pending_tool = (main_permission_pending && !main_permission_is_question)
        .then_some(session.current_tool.as_deref())
        .flatten();
    let (mut conversation, resolved_queued_prompt_ids) = parse_session_transcript(
        session.transcript_path.as_deref(),
        session.engine,
        parent.as_ref(),
        pending_tool,
        user_marker.as_deref(),
        &queued_prompts,
        &proposal_items,
    )?;

    let messages = &mut conversation.messages;
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

    for (seq, item, unresolved) in &proposal_items {
        if *unresolved
            && !messages
                .iter()
                .any(|message| matches!(message, ChatItem::Proposal { seq: existing, .. } if existing == seq))
        {
            messages.push(item.clone());
        }
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

    if !resolved_queued_prompt_ids.is_empty()
        && let Err(error) = delete_queued_prompts(session_id, &resolved_queued_prompt_ids)
    {
        tracing::warn!(
            session_id,
            %error,
            "failed to remove resolved watch-web queued prompts"
        );
    }
    Ok(Some(conversation))
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
    path: Option<&Path>,
    engine: Engine,
    parent: Option<&ParentSession>,
    pending_tool: Option<&str>,
    watch_web_user_marker: Option<&str>,
    queued_prompts: &[WatchWebQueuedPrompt],
    proposals: &[(i64, ChatItem, bool)],
) -> Result<(ConversationResponse, Vec<i64>)> {
    match engine {
        Engine::Claude => {
            let reader = match path {
                Some(path) => open_seekable_transcript(path)?,
                None => None,
            };
            let reader = reader.unwrap_or_else(|| Box::new(Cursor::new(Vec::new())));
            let (messages, background_tasks, resolved_ids) =
                parser::claude::parse_with_queued_prompts_and_proposals(
                    reader,
                    parent,
                    pending_tool,
                    watch_web_user_marker,
                    chrono::Utc::now(),
                    queued_prompts,
                    proposals,
                )?;
            Ok((
                ConversationResponse {
                    messages,
                    background_tasks,
                },
                resolved_ids,
            ))
        }
        Engine::Codex => {
            let Some(path) = path else {
                return Ok((ConversationResponse::default(), Vec::new()));
            };
            let Some(reader) = open_transcript(path, true)? else {
                return Ok((ConversationResponse::default(), Vec::new()));
            };
            Ok((
                ConversationResponse {
                    messages: parser::codex::parse(reader, parent)?,
                    background_tasks: Vec::new(),
                },
                Vec::new(),
            ))
        }
    }
}

fn proposal_chat_item(proposal: &AgentProposal) -> Result<(i64, ChatItem, bool)> {
    let body: ProposalBody = serde_json::from_str(&proposal.body)
        .with_context(|| format!("parsing proposal {} body", proposal.seq))?;
    let answer = proposal
        .answer
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .with_context(|| format!("parsing proposal {} answer", proposal.seq))?;
    Ok((
        proposal.seq,
        ChatItem::Proposal {
            seq: proposal.seq,
            title: body.title,
            summary: body.summary,
            options: body.options,
            answer,
            resolved: proposal.resolved_at.is_some(),
        },
        proposal.resolved_at.is_none(),
    ))
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
    QueuedUser {
        content: Vec<UserContentPart>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    UserWithContent {
        content: Vec<UserContentPart>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    Peer {
        direction: PeerDirection,
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_title: Option<String>,
        content: Vec<UserContentPart>,
    },
    Event {
        kind: EventKind,
        text: String,
        failed: bool,
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
    Proposal {
        seq: i64,
        title: String,
        summary: Option<String>,
        options: Vec<ProposalOption>,
        answer: Option<ProposalAnswer>,
        resolved: bool,
    },
    Permission {
        tool: String,
        description: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalOption {
    label: String,
    description: Option<String>,
    #[serde(default)]
    pros: Vec<String>,
    #[serde(default)]
    cons: Vec<String>,
    #[serde(default)]
    recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalAnswer {
    choice: Option<usize>,
    comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalBody {
    title: String,
    summary: Option<String>,
    options: Vec<ProposalOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PeerDirection {
    From,
    To,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum EventKind {
    Armyknife,
    System,
    Hook,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversationResponse {
    messages: Vec<ChatItem>,
    background_tasks: Vec<BackgroundTask>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BackgroundTask {
    pub(super) id: String,
    pub(super) description: String,
    pub(super) elapsed_seconds: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum UserContentPart {
    Text { text: String },
    Image { source: String, available: bool },
    File { name: String },
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
    use std::sync::{Arc, Mutex};

    use axum::body::{Body, to_bytes};
    use axum::http::header::CONTENT_TYPE;
    use axum::http::{Request, Response, StatusCode};
    use serde_json::json;
    use tower::ServiceExt;

    use super::{ChatItem, load_messages_with, routes_with_queued_prompts};
    use crate::commands::agent::store::{AgentProposal, WatchWebQueuedPrompt};
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

    #[test]
    fn unanswered_proposal_without_a_transcript_position_is_appended_to_conversation() {
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let proposal = AgentProposal {
            session_id: "fixture-session".to_string(),
            seq: 1,
            body: r#"{"title":"Sample proposal","summary":null,"options":[{"label":"Option A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Option B","description":null,"pros":[],"cons":[],"recommended":false}]}"#.to_string(),
            created_at,
            answer: None,
            resolved_at: None,
        };

        let conversation = super::load_conversation_with_queued_prompts(
            "fixture-session",
            |id| Ok((id == "fixture-session").then(stored_session)),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(vec![proposal]),
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
                "answer": null,
                "resolved": false,
            }]),
        );
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

    fn saved_prompt(id: i64, content: &str) -> WatchWebQueuedPrompt {
        WatchWebQueuedPrompt {
            id,
            content: content.to_string(),
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
    async fn peer_message_route_includes_the_resolved_session_title() {
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

        let router = routes_with_queued_prompts(
            move |id| {
                Ok(match id {
                    "fixture-child-id" => Some(child.clone()),
                    "fixture-parent-id" => Some(parent.clone()),
                    _ => None,
                })
            },
            |_| Ok(None),
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
                r#"{"messages":[{"type":"peer","direction":"from","session_id":"fixture-parent-id","session_title":"Fixture parent","content":[{"type":"text","text":"Synthetic peer request."}]}],"backgroundTasks":[]}"#
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
            br#"{"type":"queue-operation","operation":"enqueue","content":"Continue the fixture build."}"#,
        )
        .unwrap();

        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let router = routes_with_queued_prompts(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
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
                r#"{"messages":[{"type":"queued_user","content":[{"type":"text","text":"Continue the fixture build."}]}],"backgroundTasks":[]}"#.to_string(),
            ),
        );
    }

    #[tokio::test]
    async fn claude_message_route_returns_saved_prompts_when_no_transcript_exists() {
        let session = stored_session();
        let queued = saved_prompt(7, "Continue the fixture build.");
        let deleted_ids = Arc::new(Mutex::new(Vec::new()));
        let delete_log = Arc::clone(&deleted_ids);
        let router = routes_with_queued_prompts(
            move |_| Ok(Some(session.clone())),
            |_| Ok(Some("⟦armyknife-user:fixture-marker-cobalt⟧".to_string())),
            move |_| Ok(vec![queued.clone()]),
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
                    r#"{"messages":[{"type":"queued_user","content":[{"type":"text","text":"Continue the fixture build."}]}],"backgroundTasks":[]}"#.to_string(),
                ),
                Vec::<i64>::new(),
            ),
        );
    }

    #[tokio::test]
    async fn claude_message_route_deletes_prompts_after_marked_delivery() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            r#"{"type":"user","origin":{"kind":"peer","from":"unknown"},"timestamp":"2026-01-01T00:00:01Z","message":{"content":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nDelivered fixture request."}}"#,
        )
        .unwrap();
        let mut session = stored_session();
        session.transcript_path = Some(transcript_path);
        let queued = saved_prompt(7, "Delivered fixture request.");
        let deleted_ids = Arc::new(Mutex::new(Vec::new()));
        let delete_log = Arc::clone(&deleted_ids);
        let router = routes_with_queued_prompts(
            move |_| Ok(Some(session.clone())),
            |_| Ok(Some("⟦armyknife-user:fixture-marker-cobalt⟧".to_string())),
            move |_| Ok(vec![queued.clone()]),
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
                    r#"{"messages":[{"type":"user","text":"Delivered fixture request."}],"backgroundTasks":[]}"#.to_string(),
                ),
                vec![7],
            ),
        );
    }

    #[tokio::test]
    async fn codex_message_route_does_not_load_saved_claude_prompts() {
        let mut session = stored_session();
        session.engine = Engine::Codex;
        let router = routes_with_queued_prompts(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            |_| Err(anyhow::anyhow!("Codex must not load Claude prompts")),
            |_, _| Err(anyhow::anyhow!("Codex must not delete Claude prompts")),
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
                r#"{"messages":[],"backgroundTasks":[]}"#.to_string(),
            ),
        );
    }

    #[tokio::test]
    async fn claude_message_route_returns_running_background_tasks() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("transcript.jsonl");
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
                "timestamp": "2026-01-01T00:00:00Z",
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
        let router = routes_with_queued_prompts(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
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
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let mut body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        body["backgroundTasks"][0]["elapsedSeconds"] = json!(0);

        assert_eq!(
            body,
            json!({
                "messages": [
                    {"type": "user", "text": "Watch the fixture build."},
                    {
                        "type": "tool_summary",
                        "summary": "1 action",
                        "operations": [{"tool": "Bash", "target": "watch fixture build"}],
                    },
                ],
                "backgroundTasks": [{
                    "id": "fixture-task-id",
                    "description": "Watch the synthetic build",
                    "elapsedSeconds": 0,
                }],
            }),
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
        let router = routes_with_queued_prompts(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
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
                r#"{"messages":[{"type":"tool_in_progress","tool":"exec","target":"cargo check"}],"backgroundTasks":[]}"#.to_string(),
            )
        );
    }
}
