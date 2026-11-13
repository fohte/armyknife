use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::Body;
use axum::extract::Path as AxumPath;
use axum::extract::Query;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;
use serde_json::Value;

use crate::commands::agent::proposal::{ProposalAnswer, ProposalOption};
use crate::commands::agent::store::{
    AgentDecision, AgentProposal, AgentTodo, PermissionDeniedOperation, PermissionRequestData,
    TodoResolution, WatchWebPeerOutMessage, WatchWebQueuedPrompt,
};
use crate::commands::agent::types::{Engine, MAIN_THREAD_AGENT_KEY, Session};
use crate::commands::agent::user_turn::is_ask_user_question;

mod background_task_outputs;
mod background_tasks;
mod incremental;
mod parser;
mod proposals;
mod questions;
mod range;
mod subagent_reports;

use self::incremental::parse_session_transcript_incrementally;
use self::proposals::proposal_chat_item;
#[cfg(test)]
use self::questions::QuestionChoice;
use self::questions::{
    QuestionPrompt, attach_question_request_if_present, question_item_from_input,
};
use self::range::MessageRange;

mod cache;
use self::cache::{MessageCache, MessageCacheKey, MessageCacheRecords, RunningBackgroundTaskCache};
mod session_titles;
use self::session_titles::{parent_session, resolve_peer_session_titles};

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
type MessagesRouteState = (SessionLoader, UserMarkerLoader, RouteLoaders);
type DecisionLoader = Arc<dyn Fn(&str) -> Result<Vec<AgentDecision>> + Send + Sync>;
type QueuedPromptLoader = Arc<dyn Fn(&str) -> Result<Vec<WatchWebQueuedPrompt>> + Send + Sync>;
type PeerOutMessageLoader = Arc<dyn Fn(&str) -> Result<Vec<WatchWebPeerOutMessage>> + Send + Sync>;
type QueuedPromptDeleter = Arc<dyn Fn(&str, &[i64]) -> Result<()> + Send + Sync>;
type ProposalLoader = Arc<dyn Fn(&str) -> Result<Vec<AgentProposal>> + Send + Sync>;
type TodoLoader = Arc<dyn Fn(&str) -> Result<Vec<AgentTodo>> + Send + Sync>;
type PermissionRequestDataLoader =
    Arc<dyn Fn(&str, &str) -> Result<Option<PermissionRequestData>> + Send + Sync>;
type PermissionDeniedLoader =
    Arc<dyn Fn(&str) -> Result<Vec<PermissionDeniedOperation>> + Send + Sync>;

#[derive(Clone)]
pub(super) struct RouteLoaders {
    decisions: DecisionLoader,
    queued_prompts: QueuedPromptLoader,
    peer_out_messages: PeerOutMessageLoader,
    delete_queued_prompts: QueuedPromptDeleter,
    proposals: ProposalLoader,
    todos: TodoLoader,
    permission_request_data: PermissionRequestDataLoader,
    permission_denials: PermissionDeniedLoader,
    message_cache: MessageCache,
    running_background_task_cache: RunningBackgroundTaskCache,
}

impl RouteLoaders {
    pub(super) fn new(
        load_decisions: impl Fn(&str) -> Result<Vec<AgentDecision>> + Send + Sync + 'static,
        load_queued_prompts: impl Fn(&str) -> Result<Vec<WatchWebQueuedPrompt>> + Send + Sync + 'static,
        load_peer_out_messages: impl Fn(&str) -> Result<Vec<WatchWebPeerOutMessage>>
        + Send
        + Sync
        + 'static,
        delete_queued_prompts: impl Fn(&str, &[i64]) -> Result<()> + Send + Sync + 'static,
        load_proposals: impl Fn(&str) -> Result<Vec<AgentProposal>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            decisions: Arc::new(load_decisions),
            queued_prompts: Arc::new(load_queued_prompts),
            peer_out_messages: Arc::new(load_peer_out_messages),
            delete_queued_prompts: Arc::new(delete_queued_prompts),
            proposals: Arc::new(load_proposals),
            todos: Arc::new(|_| Ok(Vec::new())),
            permission_request_data: Arc::new(|_, _| Ok(None)),
            permission_denials: Arc::new(|_| Ok(Vec::new())),
            message_cache: MessageCache::default(),
            running_background_task_cache: RunningBackgroundTaskCache::default(),
        }
    }

    pub(super) fn with_todo_loader(
        mut self,
        load_todos: impl Fn(&str) -> Result<Vec<AgentTodo>> + Send + Sync + 'static,
    ) -> Self {
        self.todos = Arc::new(load_todos);
        self
    }

    pub(super) fn with_permission_request_data_loader(
        mut self,
        load_permission_request_data: impl Fn(&str, &str) -> Result<Option<PermissionRequestData>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.permission_request_data = Arc::new(load_permission_request_data);
        self
    }

    pub(super) fn with_permission_denied_loader(
        mut self,
        load_permission_denials: impl Fn(&str) -> Result<Vec<PermissionDeniedOperation>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.permission_denials = Arc::new(load_permission_denials);
        self
    }

    pub(super) fn running_background_task_session_ids(
        &self,
        sessions: &[Session],
    ) -> HashSet<String> {
        background_tasks::running_background_task_session_ids(
            sessions,
            &self.running_background_task_cache,
        )
    }
}

#[cfg(test)]
fn routes_with_decisions_and_queued_prompts_and_peer_out_messages(
    load_session: impl Fn(&str) -> Result<Option<Session>> + Send + Sync + 'static,
    load_user_marker: impl Fn(&str) -> Result<Option<String>> + Send + Sync + 'static,
    load_decisions: impl Fn(&str) -> Result<Vec<AgentDecision>> + Send + Sync + 'static,
    load_queued_prompts: impl Fn(&str) -> Result<Vec<WatchWebQueuedPrompt>> + Send + Sync + 'static,
    load_peer_out_messages: impl Fn(&str) -> Result<Vec<WatchWebPeerOutMessage>> + Send + Sync + 'static,
    delete_queued_prompts: impl Fn(&str, &[i64]) -> Result<()> + Send + Sync + 'static,
) -> Router {
    routes_with_proposals_and_peer_out_messages(
        load_session,
        load_user_marker,
        RouteLoaders::new(
            load_decisions,
            load_queued_prompts,
            load_peer_out_messages,
            delete_queued_prompts,
            |_| Ok(Vec::new()),
        ),
    )
}

pub(super) fn routes_with_proposals_and_peer_out_messages(
    load_session: impl Fn(&str) -> Result<Option<Session>> + Send + Sync + 'static,
    load_user_marker: impl Fn(&str) -> Result<Option<String>> + Send + Sync + 'static,
    loaders: RouteLoaders,
) -> Router {
    let load_session = Arc::new(load_session) as SessionLoader;
    Router::new()
        .route(
            "/sessions/{id}/background-tasks/{task_id}/output",
            get(background_task_outputs::get_background_task_output),
        )
        .route("/sessions/{id}/messages", get(get_messages))
        .with_state((
            load_session,
            Arc::new(load_user_marker) as UserMarkerLoader,
            loaders,
        ))
}

async fn get_messages(
    State((load_session, load_user_marker, loaders)): State<MessagesRouteState>,
    AxumPath(session_id): AxumPath<String>,
    Query(range): Query<MessageRange>,
) -> Response {
    if !range.is_valid() {
        return StatusCode::BAD_REQUEST.into_response();
    }

    match tokio::task::spawn_blocking(move || {
        load_conversation_with_queued_prompts(
            &session_id,
            |id| load_session(id),
            |id| load_user_marker(id),
            &loaders,
        )
    })
    .await
    {
        Ok(Ok(Some(mut conversation))) => {
            conversation.apply_range(range);
            match serde_json::to_vec(&conversation) {
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
            }
        }
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

fn load_conversation_with_queued_prompts(
    session_id: &str,
    mut load_session: impl FnMut(&str) -> Result<Option<Session>>,
    load_user_marker: impl FnOnce(&str) -> Result<Option<String>>,
    loaders: &RouteLoaders,
) -> Result<Option<ConversationResponse>> {
    let Some(session) = load_session(session_id)? else {
        return Ok(None);
    };
    let user_marker = if session.engine == Engine::Claude {
        load_user_marker(session_id)?
    } else {
        None
    };
    let decisions = (loaders.decisions)(session_id)?;
    let queued_prompts = if session.engine == Engine::Claude {
        (loaders.queued_prompts)(session_id)?
    } else {
        Vec::new()
    };
    let peer_out_messages = (loaders.peer_out_messages)(session_id)?;
    let proposals = (loaders.proposals)(session_id)?;
    let proposal_items = proposals
        .iter()
        .map(proposal_chat_item)
        .collect::<Result<Vec<_>>>()?;
    let todos = (loaders.todos)(session_id)?;
    let permission_denials = (loaders.permission_denials)(session_id)?;
    let parent = parent_session(&session, &mut load_session)?;
    let main_permission_pending = session
        .pending_permission_agent_ids
        .contains(MAIN_THREAD_AGENT_KEY);
    let main_permission_request_id = session
        .pending_permission_request_ids
        .get(MAIN_THREAD_AGENT_KEY)
        .cloned();
    let main_permission_data = main_permission_request_id
        .as_deref()
        .map(|request_id| (loaders.permission_request_data)(session_id, request_id))
        .transpose()?
        .flatten();
    let main_permission_is_question = main_permission_data
        .as_ref()
        .and_then(|data| data.tool_name.as_deref())
        == Some("AskUserQuestion")
        || (main_permission_data.is_none() && is_ask_user_question(&session));
    let pending_tool = if main_permission_pending && !main_permission_is_question {
        main_permission_data
            .as_ref()
            .and_then(permission_request_pending_tool)
            .or_else(|| session.current_tool.clone())
    } else {
        None
    };
    let cache_key = MessageCacheKey::new(
        &session,
        parent.clone(),
        user_marker.clone(),
        decisions.clone(),
        queued_prompts.clone(),
        peer_out_messages.clone(),
        MessageCacheRecords {
            proposals: proposals.clone(),
            todos: todos.clone(),
            permission_denials: permission_denials.clone(),
        },
    )?;
    if let Some(mut cached) = loaders.message_cache.get(session_id, &cache_key)? {
        let elapsed = cached.cached_at.elapsed().as_secs();
        for task in &mut cached.conversation.background_tasks {
            task.elapsed_seconds = task.elapsed_seconds.saturating_add(elapsed);
        }
        cache::refresh_image_availability(&mut cached.conversation);
        if let Some(path) = session.transcript_path.as_deref() {
            subagent_reports::add_descriptions(path, &mut cached.conversation.messages);
        }
        attach_question_request_if_present(
            &mut cached.conversation.messages,
            main_permission_request_id.as_deref(),
            main_permission_data.as_ref(),
        );
        resolve_peer_session_titles(&mut cached.conversation.messages, &mut load_session)?;
        cache::delete_resolved_queued_prompts(
            loaders,
            session_id,
            &cached.resolved_queued_prompt_ids,
        );
        return Ok(Some(cached.conversation));
    }
    let transcript_input = TranscriptInput {
        path: session.transcript_path.as_deref(),
        engine: session.engine,
        additions: parser::claude::ParserAdditions {
            parent: parent.as_ref(),
            pending_tool: pending_tool.as_deref(),
            user_marker: user_marker.as_deref(),
            decisions: &decisions,
            queued_prompts: &queued_prompts,
            peer_out_messages: &peer_out_messages,
            proposals: &proposal_items,
            todos: &todos,
        },
    };
    let (mut conversation, resolved_queued_prompt_ids) = parse_session_transcript_incrementally(
        session_id,
        transcript_input,
        cache_key.transcript_fingerprint.clone(),
        &loaders.message_cache,
    )?;
    resolve_peer_session_titles(&mut conversation.messages, &mut load_session)?;

    attach_question_request_if_present(
        &mut conversation.messages,
        main_permission_request_id.as_deref(),
        main_permission_data.as_ref(),
    );

    for message in &mut conversation.messages {
        if let ChatItem::Permission { request_id, .. } = message {
            *request_id = main_permission_request_id.clone();
        }
    }

    let messages = &mut conversation.messages;
    if main_permission_pending
        && !main_permission_is_question
        && !messages
            .iter()
            .any(|message| matches!(message, ChatItem::Permission { .. }))
    {
        messages.push(ChatItem::Permission {
            request_id: main_permission_request_id.clone(),
            tool: main_permission_data
                .as_ref()
                .and_then(permission_request_pending_tool)
                .or_else(|| session.current_tool.clone())
                .unwrap_or_else(|| "Permission required".to_string()),
            description: "Waiting for permission".to_string(),
        });
    }

    for proposal in &proposal_items {
        if !messages
            .iter()
            .any(|message| matches!(message, ChatItem::Proposal { seq: existing, .. } if existing == &proposal.seq))
        {
            messages.push(proposal.item.clone());
        }
    }

    for operation in permission_denials {
        if !messages.iter().any(|message| {
            matches!(message, ChatItem::PermissionDenied { seq, .. } if *seq == operation.seq)
        }) {
            let retryable = operation.has_classifier_verdict();
            messages.push(ChatItem::PermissionDenied {
                seq: operation.seq,
                tool_use_id: operation.tool_use_id,
                tool: operation.tool_name.clone(),
                input_summary: permission_denied_input_summary(
                    &operation.tool_name,
                    &operation.tool_input,
                ),
                reason: operation.reason.clone(),
                retryable,
                resolved: operation.resolved_at.is_some(),
            });
        }
    }

    for agent_id in session
        .pending_permission_agent_ids
        .iter()
        .filter(|agent_id| agent_id.as_str() != MAIN_THREAD_AGENT_KEY)
    {
        let Some(request_id) = session
            .pending_permission_request_ids
            .get(agent_id)
            .cloned()
        else {
            continue;
        };
        let permission_data = (loaders.permission_request_data)(session_id, &request_id)?;
        if permission_data
            .as_ref()
            .is_some_and(|data| data.tool_name.as_deref() == Some("AskUserQuestion"))
        {
            let Some(data) = permission_data.as_ref() else {
                continue;
            };
            if let Some(question) = question_item_from_input(Some(request_id), &data.tool_input) {
                messages.push(question);
            }
            continue;
        }
        messages.push(ChatItem::Permission {
            request_id: Some(request_id),
            tool: format!(
                "Subagent {agent_id}: {}",
                permission_data
                    .and_then(|data| data.tool_name)
                    .unwrap_or_else(|| "Permission required".to_string())
            ),
            description: "Waiting for permission approval.".to_string(),
        });
    }

    conversation.update_metadata();

    cache::delete_resolved_queued_prompts(loaders, session_id, &resolved_queued_prompt_ids);
    loaders.message_cache.insert(
        session_id,
        cache_key,
        conversation.clone(),
        resolved_queued_prompt_ids,
    );
    Ok(Some(conversation))
}

fn permission_request_pending_tool(data: &PermissionRequestData) -> Option<String> {
    let tool_name = data.tool_name.as_deref()?;
    let detail = permission_request_tool_detail(&data.tool_input);
    Some(match detail {
        Some(detail) => format!("{tool_name}({detail})"),
        None => tool_name.to_string(),
    })
}

fn permission_denied_input_summary(tool_name: &str, input: &Value) -> String {
    let detail = match tool_name {
        "Bash" => input.get("command"),
        "Read" | "Write" | "Edit" | "NotebookEdit" => input.get("file_path"),
        "Grep" | "Glob" => input.get("pattern"),
        _ => None,
    }
    .and_then(Value::as_str)
    .map(str::to_string)
    .unwrap_or_else(|| serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string()));
    detail.chars().take(200).collect()
}

fn permission_request_tool_detail(tool_input: &Value) -> Option<&str> {
    ["command", "file_path", "pattern"]
        .into_iter()
        .find_map(|key| tool_input.get(key).and_then(Value::as_str))
}

pub(in crate::commands::agent::watch_web::messages) struct TranscriptInput<'a> {
    path: Option<&'a Path>,
    engine: Engine,
    additions: parser::claude::ParserAdditions<'a>,
}

pub(in crate::commands::agent::watch_web::messages) fn parse_session_transcript(
    input: TranscriptInput<'_>,
) -> Result<(ConversationResponse, Vec<i64>)> {
    incremental::parse_session_transcript(input)
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
        timestamp: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    QueuedUser {
        content: Vec<UserContentPart>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    UserWithContent {
        content: Vec<UserContentPart>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    Peer {
        direction: PeerDirection,
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_title: Option<String>,
        content: Vec<UserContentPart>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        #[serde(skip_serializing_if = "is_false")]
        failed: bool,
    },
    SubagentReport {
        agent_id: String,
        agent_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        text: String,
    },
    Event {
        kind: EventKind,
        text: String,
        failed: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
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
        #[serde(rename = "requestId", skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        questions: Vec<QuestionPrompt>,
    },
    Proposal {
        seq: i64,
        title: String,
        summary: Option<String>,
        options: Vec<ProposalOption>,
        answer: Option<ProposalAnswer>,
        resolved: bool,
    },
    Todo {
        seq: i64,
        title: String,
        detail: Option<String>,
        completed: bool,
        resolution: Option<TodoResolution>,
        comment: Option<String>,
    },
    Permission {
        #[serde(rename = "requestId", skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        tool: String,
        description: String,
    },
    PermissionDenied {
        seq: i64,
        #[serde(rename = "toolUseId", skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
        tool: String,
        #[serde(rename = "inputSummary")]
        input_summary: String,
        reason: Option<String>,
        retryable: bool,
        resolved: bool,
    },
    Decision {
        seq: i64,
        title: String,
        why: String,
        considered: Vec<String>,
        #[serde(rename = "easyToChange")]
        easy_to_change: String,
        review: Option<String>,
        #[serde(rename = "createdAt")]
        created_at: String,
    },
}

fn is_false(value: &bool) -> bool {
    !value
}

impl From<AgentDecision> for ChatItem {
    fn from(decision: AgentDecision) -> Self {
        Self::Decision {
            seq: decision.seq,
            title: decision.title,
            why: decision.why,
            considered: decision.considered,
            easy_to_change: decision.easy_to_change,
            review: decision.review,
            created_at: decision.created_at,
        }
    }
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
    CompactBoundary,
    System,
    Hook,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversationResponse {
    messages: Vec<ChatItem>,
    background_tasks: Vec<BackgroundTask>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    skills: Vec<SkillSuggestion>,
    start: usize,
    total: usize,
    decisions: Vec<ChatItem>,
    has_unanswered_proposal: bool,
    active_question_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stable_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stable_end: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stable_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillSuggestion {
    pub(super) name: String,
    pub(super) description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BackgroundTask {
    pub(super) id: String,
    pub(super) description: String,
    pub(super) elapsed_seconds: u64,
    pub(super) kind: BackgroundTaskKind,
    #[serde(skip)]
    pub(super) output_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) agent_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BackgroundTaskKind {
    Subagent,
    BackgroundTask,
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
    failed: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct ParentSession {
    session_id: String,
    title: String,
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
    use rstest::rstest;
    use serde_json::json;
    use tower::ServiceExt;

    use super::cache::TranscriptChange;
    use super::{
        ChatItem, QuestionChoice, QuestionPrompt, RouteLoaders,
        load_conversation_with_queued_prompts, load_messages_with,
        routes_with_decisions_and_queued_prompts_and_peer_out_messages,
        routes_with_proposals_and_peer_out_messages,
    };
    use crate::commands::agent::store::{
        AgentDecision, AgentProposal, AgentTodo, PermissionDeniedOperation, PermissionRequestData,
        TodoResolution, WatchWebPeerOutMessage, WatchWebQueuedPrompt,
    };
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

    #[rstest]
    #[case::classifier_verdict(Some("[Sensitive Operation]"), true, "[Sensitive Operation]")]
    #[case::missing_verdict(
        Some("Auto mode could not evaluate this action and is blocking it for safety."),
        false,
        "Auto mode could not evaluate this action and is blocking it for safety."
    )]
    #[tokio::test]
    async fn messages_route_shows_permission_denial_and_retryability(
        #[case] reason: Option<&str>,
        #[case] retryable: bool,
        #[case] expected_reason: &str,
    ) {
        let operation = PermissionDeniedOperation {
            session_id: "fixture-session".to_string(),
            seq: 1,
            tool_use_id: Some("tool-call-fixture".to_string()),
            tool_name: "Bash".to_string(),
            tool_input: json!({ "command": "printf fixture" }),
            reason: reason.map(str::to_string),
            created_at: chrono::Utc::now(),
            resolved_at: None,
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

        assert_eq!(
            response,
            (
                StatusCode::OK,
                Some("application/json".to_string()),
                json!({
                    "messages": [{
                        "type": "permission_denied",
                        "seq": 1,
                        "toolUseId": "tool-call-fixture",
                        "tool": "Bash",
                        "inputSummary": "printf fixture",
                        "reason": expected_reason,
                        "retryable": retryable,
                        "resolved": false,
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
    async fn messages_route_returns_session_skill_suggestions(#[case] use_codex_transcript: bool) {
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

        let mut session = stored_session();
        session.engine = if use_codex_transcript {
            Engine::Codex
        } else {
            Engine::Claude
        };
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
            proposal.answer = Some(r#"{"choice":0,"comment":"Keep option A."}"#.to_string());
            proposal.resolved_at = Some(created_at);
        }
        let resolved = message_response(&router).await;

        assert_eq!(
            (unanswered, resolved),
            (
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[{"type":"proposal","seq":1,"title":"Sample proposal","summary":null,"options":[{"label":"Option A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Option B","description":null,"pros":[],"cons":[],"recommended":false}],"answer":null,"resolved":false}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":true,"activeQuestionIndex":null})
                ),
                (
                    StatusCode::OK,
                    Some("application/json".to_string()),
                    json!({"messages":[{"type":"proposal","seq":1,"title":"Sample proposal","summary":null,"options":[{"label":"Option A","description":null,"pros":[],"cons":[],"recommended":false},{"label":"Option B","description":null,"pros":[],"cons":[],"recommended":false}],"answer":{"choice":0,"comment":"Keep option A."},"resolved":true}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null})
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
            }]),
        );
    }

    #[tokio::test]
    async fn messages_route_includes_recorded_todos_and_completion_state() {
        let epoch = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let session = stored_session();
        let todos = vec![
            AgentTodo {
                session_id: "fixture-session".to_string(),
                seq: 1,
                title: "Check the page".to_string(),
                detail: Some("Use a phone".to_string()),
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
                created_at: epoch,
                completed_at: Some(epoch),
                resolution: Some(TodoResolution::Completed),
                comment: Some("Done".to_string()),
            },
        ];
        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_todo_loader(move |_| Ok(todos.clone()));
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
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/sessions/fixture-session/messages")
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
            review: None,
            reviewed_at: None,
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
                            "review": null,
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
                        },
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
                        "review": null,
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
                    review: None,
                    reviewed_at: None,
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
                            "review": null,
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
                        "review": null,
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
                    review: None,
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
                r#"{"messages":[{"type":"assistant","text":"Before the decision."},{"type":"decision","seq":1,"title":"Choose the fixture path","why":"The fixture remains reversible.","considered":["Choose the other fixture path"],"easyToChange":"Change one fixture value.","review":null,"createdAt":"2026-11-11T12:00:00Z"},{"type":"assistant","text":"After the decision."}],"backgroundTasks":[],"start":0,"total":3,"decisions":[{"type":"decision","seq":1,"title":"Choose the fixture path","why":"The fixture remains reversible.","considered":["Choose the other fixture path"],"easyToChange":"Change one fixture value.","review":null,"createdAt":"2026-11-11T12:00:00Z"}],"hasUnansweredProposal":false,"activeQuestionIndex":null}"#.to_string(),
            ),
        );
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

    #[test]
    fn pending_permission_fallback_does_not_reuse_the_last_assistant_message() {
        let mut session = stored_session();
        session.pending_permission_agent_ids = BTreeSet::from([MAIN_THREAD_AGENT_KEY.to_string()]);
        session.pending_permission_request_ids = BTreeMap::from([(
            MAIN_THREAD_AGENT_KEY.to_string(),
            "permission-request-main".to_string(),
        )]);
        session.current_tool = Some("Bash(cargo test -p sample)".to_string());
        session.last_message = Some("Run the package tests".to_string());

        assert_eq!(
            load_messages_with("fixture-session", |id| {
                Ok((id == "fixture-session").then(|| session.clone()))
            })
            .unwrap(),
            Some(vec![ChatItem::Permission {
                request_id: Some("permission-request-main".to_string()),
                tool: "Bash(cargo test -p sample)".to_string(),
                description: "Waiting for permission".to_string(),
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
            })
            .unwrap(),
            json!({
                "type": "permission",
                "requestId": "permission-request-example",
                "tool": "Bash",
                "description": "Waiting for permission",
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
                },
            ])
        );
    }

    #[test]
    fn pending_permission_cards_target_each_agent_and_omit_question_requests() {
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
        .with_permission_request_data_loader(|_, request_id| {
            let tool_name = match request_id {
                "permission-request-a" => "Write",
                "permission-request-question" => "AskUserQuestion",
                _ => return Ok(None),
            };
            Ok(Some(PermissionRequestData {
                tool_name: Some(tool_name.to_string()),
                tool_input: json!({}),
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
                    tool: "Bash(example command)".to_string(),
                    description: "Waiting for permission".to_string(),
                },
                ChatItem::Permission {
                    request_id: Some("permission-request-a".to_string()),
                    tool: "Subagent agent-fixture-a: Write".to_string(),
                    description: "Waiting for permission approval.".to_string(),
                },
            ],
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
                    r#"{"messages":[{"type":"queued_user","content":[{"type":"text","text":"Continue the fixture build."}],"timestamp":"2026-01-01T00:00:00+00:00"}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null}"#.to_string(),
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
                    r#"{"messages":[{"type":"user","text":"Delivered fixture request.","timestamp":"2026-01-01T00:00:01+00:00"}],"backgroundTasks":[],"start":0,"total":1,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null}"#.to_string(),
                ),
                vec![7],
            ),
        );
    }

    #[tokio::test]
    async fn codex_message_route_does_not_load_saved_claude_prompts() {
        let mut session = stored_session();
        session.engine = Engine::Codex;
        let router = routes_with_decisions_and_queued_prompts_and_peer_out_messages(
            move |_| Ok(Some(session.clone())),
            |_| Ok(None),
            |_| Ok(Vec::new()),
            |_| Err(anyhow::anyhow!("Codex must not load Claude prompts")),
            |_| Ok(Vec::new()),
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
                r#"{"messages":[],"backgroundTasks":[],"start":0,"total":0,"decisions":[],"hasUnansweredProposal":false,"activeQuestionIndex":null}"#.to_string(),
            ),
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
                    {"type": "user", "text": "Watch the fixture build."},
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
}
