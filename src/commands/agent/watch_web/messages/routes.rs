use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
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

use crate::commands::agent::store::{
    AgentDecision, AgentFile, AgentProposal, AgentTodo, PermissionDeniedOperation,
    PermissionRequestData, WatchWebPeerOutMessage, WatchWebQueuedPrompt,
};
use crate::commands::agent::types::Session;

use super::cache::MessageCache;
use super::conversation::{
    load_conversation_with_initial_tail_window, load_conversation_with_queued_prompts,
};
use super::range::MessageRange;
use super::{BackgroundTask, background_task_outputs, question_preview};

pub(super) type SessionLoader = Arc<dyn Fn(&str) -> Result<Option<Session>> + Send + Sync>;
pub(super) type UserMarkerLoader = Arc<dyn Fn(&str) -> Result<Option<String>> + Send + Sync>;
pub(super) type MessagesRouteState = (SessionLoader, UserMarkerLoader, RouteLoaders);
pub(super) type DecisionLoader = Arc<dyn Fn(&str) -> Result<Vec<AgentDecision>> + Send + Sync>;
pub(super) type QueuedPromptLoader =
    Arc<dyn Fn(&str) -> Result<Vec<WatchWebQueuedPrompt>> + Send + Sync>;
pub(super) type PeerOutMessageLoader =
    Arc<dyn Fn(&str) -> Result<Vec<WatchWebPeerOutMessage>> + Send + Sync>;
pub(super) type QueuedPromptUpdater = Arc<dyn Fn(&str, &[i64]) -> Result<()> + Send + Sync>;
pub(super) type ProposalLoader = Arc<dyn Fn(&str) -> Result<Vec<AgentProposal>> + Send + Sync>;
pub(super) type TodoLoader = Arc<dyn Fn(&str) -> Result<Vec<AgentTodo>> + Send + Sync>;
pub(super) type PermissionRequestDataLoader =
    Arc<dyn Fn(&str, &str) -> Result<Option<PermissionRequestData>> + Send + Sync>;
pub(super) type PermissionDeniedLoader =
    Arc<dyn Fn(&str) -> Result<Vec<PermissionDeniedOperation>> + Send + Sync>;
pub(super) type AgentFileLoader = Arc<dyn Fn(&str) -> Result<Vec<AgentFile>> + Send + Sync>;
pub(super) type AgentFileStorageRootLoader = Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>;
pub(super) type BackgroundTaskLoader =
    Arc<dyn Fn(&str) -> Result<Vec<BackgroundTask>> + Send + Sync>;
type RepoIdentityLoader = Arc<dyn Fn(&Path) -> Option<(String, String)> + Send + Sync>;
type VibeConfigLoader = Arc<dyn Fn() -> crate::shared::config::Config + Send + Sync>;

#[derive(Clone)]
pub(in crate::commands::agent::watch_web) struct RouteLoaders {
    pub(super) decisions: DecisionLoader,
    pub(super) queued_prompts: QueuedPromptLoader,
    pub(super) peer_out_messages: PeerOutMessageLoader,
    pub(super) mark_queued_prompts_resolved: QueuedPromptUpdater,
    pub(super) proposals: ProposalLoader,
    pub(super) todos: TodoLoader,
    pub(super) permission_request_data: PermissionRequestDataLoader,
    pub(super) permission_denials: PermissionDeniedLoader,
    pub(super) agent_files: AgentFileLoader,
    pub(super) agent_file_storage_root: AgentFileStorageRootLoader,
    pub(super) background_tasks: BackgroundTaskLoader,
    vibe_config: Option<VibeConfigLoader>,
    repo_identity: Option<RepoIdentityLoader>,
    pub(super) message_cache: MessageCache,
    pub(super) question_preview_cache: question_preview::QuestionPreviewCache,
}

impl RouteLoaders {
    pub(in crate::commands::agent::watch_web) fn new(
        load_decisions: impl Fn(&str) -> Result<Vec<AgentDecision>> + Send + Sync + 'static,
        load_queued_prompts: impl Fn(&str) -> Result<Vec<WatchWebQueuedPrompt>> + Send + Sync + 'static,
        load_peer_out_messages: impl Fn(&str) -> Result<Vec<WatchWebPeerOutMessage>>
        + Send
        + Sync
        + 'static,
        mark_queued_prompts_resolved: impl Fn(&str, &[i64]) -> Result<()> + Send + Sync + 'static,
        load_proposals: impl Fn(&str) -> Result<Vec<AgentProposal>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            decisions: Arc::new(load_decisions),
            queued_prompts: Arc::new(load_queued_prompts),
            peer_out_messages: Arc::new(load_peer_out_messages),
            mark_queued_prompts_resolved: Arc::new(mark_queued_prompts_resolved),
            proposals: Arc::new(load_proposals),
            todos: Arc::new(|_| Ok(Vec::new())),
            permission_request_data: Arc::new(|_, _| Ok(None)),
            permission_denials: Arc::new(|_| Ok(Vec::new())),
            agent_files: Arc::new(|_| Ok(Vec::new())),
            agent_file_storage_root: Arc::new(|| None),
            background_tasks: Arc::new(|_| Ok(Vec::new())),
            vibe_config: None,
            repo_identity: None,
            message_cache: MessageCache::default(),
            question_preview_cache: question_preview::QuestionPreviewCache::default(),
        }
    }

    pub(in crate::commands::agent::watch_web) fn with_todo_loader(
        mut self,
        load_todos: impl Fn(&str) -> Result<Vec<AgentTodo>> + Send + Sync + 'static,
    ) -> Self {
        self.todos = Arc::new(load_todos);
        self
    }

    pub(in crate::commands::agent::watch_web) fn with_permission_request_data_loader(
        mut self,
        load_permission_request_data: impl Fn(&str, &str) -> Result<Option<PermissionRequestData>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.permission_request_data = Arc::new(load_permission_request_data);
        self
    }

    pub(in crate::commands::agent::watch_web) fn with_permission_denied_loader(
        mut self,
        load_permission_denials: impl Fn(&str) -> Result<Vec<PermissionDeniedOperation>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.permission_denials = Arc::new(load_permission_denials);
        self
    }

    pub(in crate::commands::agent::watch_web) fn with_agent_file_loader(
        mut self,
        load_agent_files: impl Fn(&str) -> Result<Vec<AgentFile>> + Send + Sync + 'static,
    ) -> Self {
        self.agent_files = Arc::new(load_agent_files);
        self
    }

    pub(in crate::commands::agent::watch_web) fn with_agent_file_storage_root_loader(
        mut self,
        load_storage_root: impl Fn() -> Option<PathBuf> + Send + Sync + 'static,
    ) -> Self {
        self.agent_file_storage_root = Arc::new(load_storage_root);
        self
    }

    pub(in crate::commands::agent::watch_web) fn with_background_task_loader(
        mut self,
        load_background_tasks: impl Fn(&str) -> Result<Vec<BackgroundTask>> + Send + Sync + 'static,
    ) -> Self {
        self.background_tasks = Arc::new(load_background_tasks);
        self
    }

    pub(in crate::commands::agent::watch_web) fn with_vibe_context(
        mut self,
        config: impl Fn() -> crate::shared::config::Config + Send + Sync + 'static,
        repo_identity: impl Fn(&Path) -> Option<(String, String)> + Send + Sync + 'static,
    ) -> Self {
        self.vibe_config = Some(Arc::new(config));
        self.repo_identity = Some(Arc::new(repo_identity));
        self
    }

    pub(super) fn background_tasks(&self, session_id: &str) -> Result<Vec<BackgroundTask>> {
        (self.background_tasks)(session_id)
    }
}

pub(in crate::commands::agent::watch_web) fn routes_with_proposals_and_peer_out_messages(
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
    let tail_window_limit = range.initial_tail_window_limit();

    match tokio::task::spawn_blocking(move || -> Result<Option<(
        super::ConversationResponse,
        Option<crate::commands::agent::vibe::VibeState>,
    )>> {
        let Some(session) = load_session(&session_id)? else {
            return Ok(None);
        };
        let vibe_mode = match (&loaders.vibe_config, &loaders.repo_identity) {
            (Some(config_loader), Some(repo_identity)) => {
                let config = config_loader();
                let repo = repo_identity(&session.cwd);
                let identity = repo
                    .as_ref()
                    .map(|(owner, repo)| (owner.as_str(), repo.as_str()));
                let initial_state = crate::commands::agent::vibe::state_for(
                    &session,
                    identity,
                    &config,
                    &[],
                );
                if session.vibe_enabled
                    || !session.launched_with_vibe
                    || matches!(
                        initial_state,
                        crate::commands::agent::vibe::VibeState::RepoDisabled
                    )
                {
                    Some(initial_state)
                } else {
                    crate::commands::agent::vibe::state_for_with_ancestor_loader(
                        &session,
                        identity,
                        &config,
                        |ancestor_id| load_session(ancestor_id),
                    )
                    .ok()
                }
            }
            _ => None,
        };
        let conversation = match tail_window_limit {
            Some(limit) => load_conversation_with_initial_tail_window(
                &session_id,
                |id| load_session(id),
                |id| load_user_marker(id),
                &loaders,
                limit,
            ),
            None => load_conversation_with_queued_prompts(
                &session_id,
                |id| load_session(id),
                |id| load_user_marker(id),
                &loaders,
            ),
        }?;
        Ok(conversation.map(|conversation| (conversation, vibe_mode)))
    })
    .await
    {
        Ok(Ok(Some((mut conversation, vibe_mode)))) => {
            conversation.apply_range(range);
            match serde_json::to_vec(&ConversationApiResponse {
                conversation: &conversation,
                vibe_mode,
            }) {
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversationApiResponse<'a> {
    #[serde(flatten)]
    conversation: &'a super::ConversationResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    vibe_mode: Option<crate::commands::agent::vibe::VibeState>,
}
