use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::Path as AxumPath;
use axum::extract::Query;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::commands::agent::store::{
    AgentDecision, AgentFile, AgentProposal, AgentTodo, PermissionDeniedOperation,
    PermissionRequestData, WatchWebPeerOutMessage, WatchWebQueuedPrompt,
};
use crate::commands::agent::types::{Engine, Session};

use super::cache::MessageCache;
use super::conversation::{
    load_conversation_with_initial_tail_window, load_conversation_with_queued_prompts,
    load_conversation_with_queued_prompts_for_range,
};
use super::range::MessageRange;
use super::{BackgroundTask, SkillSuggestion, background_task_outputs, question_preview};

pub(super) type SessionLoader = Arc<dyn Fn(&str) -> Result<Option<Session>> + Send + Sync>;
pub(super) type SessionListLoader = Arc<dyn Fn() -> Result<Vec<Session>> + Send + Sync>;
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
pub(in crate::commands::agent::watch_web) type BackgroundTaskOutputEventStream =
    Pin<Box<dyn Stream<Item = notify::Result<notify::Event>> + Send>>;
type BackgroundTaskOutputEventStreamFactory =
    Arc<dyn Fn(PathBuf) -> Result<BackgroundTaskOutputEventStream> + Send + Sync>;
type RepoIdentityLoader = Arc<dyn Fn(&Path) -> Option<(String, String)> + Send + Sync>;
type VibeConfigLoader = Arc<dyn Fn() -> crate::shared::config::Config + Send + Sync>;
type SkillListingCache = Arc<Mutex<HashMap<String, CachedSkillListing>>>;

struct CachedSkillListing {
    fingerprint: String,
    listing: Option<Vec<super::SkillSuggestion>>,
}

#[derive(Deserialize)]
struct SkillSuggestionsQuery {
    engine: Engine,
    directory: Option<PathBuf>,
}

#[derive(Serialize)]
struct SkillSuggestionsResponse {
    skills: Vec<SkillSuggestion>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessagesQuery {
    limit: Option<usize>,
    before: Option<usize>,
    from: Option<usize>,
    stable_start: Option<usize>,
    stable_end: Option<usize>,
    stable_digest: Option<String>,
    epoch: Option<String>,
    older_cursor: Option<String>,
    pending_prompt_ids: Option<String>,
}

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
    pub(super) background_task_output_event_stream: BackgroundTaskOutputEventStreamFactory,
    pub(super) shutdown_signal: Option<watch::Receiver<bool>>,
    session_list: SessionListLoader,
    skill_listing_cache: SkillListingCache,
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
        Self::new_with_background_task_output_event_stream(
            load_decisions,
            load_queued_prompts,
            load_peer_out_messages,
            mark_queued_prompts_resolved,
            load_proposals,
            |path| {
                Ok(Box::pin(background_task_outputs::watch_output_events(
                    path,
                )?))
            },
        )
    }

    pub(in crate::commands::agent::watch_web) fn new_with_background_task_output_event_stream(
        load_decisions: impl Fn(&str) -> Result<Vec<AgentDecision>> + Send + Sync + 'static,
        load_queued_prompts: impl Fn(&str) -> Result<Vec<WatchWebQueuedPrompt>> + Send + Sync + 'static,
        load_peer_out_messages: impl Fn(&str) -> Result<Vec<WatchWebPeerOutMessage>>
        + Send
        + Sync
        + 'static,
        mark_queued_prompts_resolved: impl Fn(&str, &[i64]) -> Result<()> + Send + Sync + 'static,
        load_proposals: impl Fn(&str) -> Result<Vec<AgentProposal>> + Send + Sync + 'static,
        create_event_stream: impl Fn(PathBuf) -> Result<BackgroundTaskOutputEventStream>
        + Send
        + Sync
        + 'static,
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
            background_task_output_event_stream: Arc::new(create_event_stream),
            shutdown_signal: None,
            session_list: Arc::new(|| Ok(Vec::new())),
            skill_listing_cache: Arc::new(Mutex::new(HashMap::new())),
            vibe_config: None,
            repo_identity: None,
            message_cache: MessageCache::default(),
            question_preview_cache: question_preview::QuestionPreviewCache::default(),
        }
    }

    pub(in crate::commands::agent::watch_web) fn with_shutdown_signal(
        mut self,
        shutdown_signal: watch::Receiver<bool>,
    ) -> Self {
        self.shutdown_signal = Some(shutdown_signal);
        self
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

    pub(in crate::commands::agent::watch_web) fn with_session_list_loader(
        mut self,
        load_sessions: impl Fn() -> Result<Vec<Session>> + Send + Sync + 'static,
    ) -> Self {
        self.session_list = Arc::new(load_sessions);
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

    pub(super) fn sessions(&self) -> Result<Vec<Session>> {
        (self.session_list)()
    }

    pub(super) fn cached_skill_listing(
        &self,
        session: &Session,
        transcript_fingerprint: &str,
        load_listing: impl FnOnce() -> Result<Option<Vec<super::SkillSuggestion>>>,
    ) -> Result<Option<Vec<super::SkillSuggestion>>> {
        let fingerprint = serde_json::to_string(&(
            session.engine,
            &session.cwd,
            &session.transcript_path,
            session.created_at,
            session.updated_at,
            transcript_fingerprint,
        ))?;
        let mut cache = self
            .skill_listing_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cached) = cache.get(&session.session_id)
            && cached.fingerprint == fingerprint
        {
            return Ok(cached.listing.clone());
        }
        drop(cache);

        let listing = load_listing()?;
        cache = self
            .skill_listing_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.insert(
            session.session_id.clone(),
            CachedSkillListing {
                fingerprint,
                listing: listing.clone(),
            },
        );
        Ok(listing)
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
            "/sessions/{id}/background-tasks/{task_id}/output/events",
            get(background_task_outputs::get_background_task_output_events),
        )
        .route("/sessions/{id}/messages", get(get_messages))
        .route("/skill-suggestions", get(get_skill_suggestions))
        .with_state((
            load_session,
            Arc::new(load_user_marker) as UserMarkerLoader,
            loaders,
        ))
}

async fn get_skill_suggestions(
    State((_, _, loaders)): State<MessagesRouteState>,
    Query(query): Query<SkillSuggestionsQuery>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        super::skill_suggestions::find_from_recent_sessions(
            query.engine,
            query.directory.as_deref(),
            None,
            &loaders,
        )
    })
    .await;
    match result {
        Ok(Ok(skills)) => Json(SkillSuggestionsResponse { skills }).into_response(),
        Ok(Err(error)) => {
            tracing::warn!(%error, "failed to find skill suggestions");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        Err(error) => {
            tracing::warn!(%error, "skill suggestions task failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_messages(
    State((load_session, load_user_marker, loaders)): State<MessagesRouteState>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<MessagesQuery>,
) -> Response {
    let range = MessageRange {
        limit: query.limit,
        before: query.before,
        from: query.from,
        stable_start: query.stable_start,
        stable_end: query.stable_end,
        stable_digest: query.stable_digest,
        epoch: query.epoch,
        older_cursor: query.older_cursor,
    };
    if !range.is_valid() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mut pending_prompt_ids = Vec::new();
    let mut seen_prompt_ids = HashSet::new();
    for message_id in query
        .pending_prompt_ids
        .as_deref()
        .into_iter()
        .flat_map(|ids| ids.split(','))
        .filter(|id| !id.is_empty())
    {
        let message_id = message_id.to_owned();
        if seen_prompt_ids.insert(message_id.clone()) {
            pending_prompt_ids.push(message_id);
        }
    }
    let tail_window_limit = range.initial_tail_window_limit();
    let requested_epoch = range.requested_epoch().map(str::to_owned);
    let older_page_limit = range.older_page_limit();
    let require_full_history = range.requires_full_history();

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
            None if requested_epoch.is_none() && !require_full_history => {
                load_conversation_with_queued_prompts(
                    &session_id,
                    |id| load_session(id),
                    |id| load_user_marker(id),
                    &loaders,
                )
            }
            None => load_conversation_with_queued_prompts_for_range(
                &session_id,
                |id| load_session(id),
                |id| load_user_marker(id),
                &loaders,
                requested_epoch.as_deref(),
                older_page_limit,
                require_full_history,
            ),
        }?;
        let Some(mut conversation) = conversation else {
            return Ok(None);
        };
        if pending_prompt_ids.is_empty() {
            conversation.confirmed_queued_prompt_ids.clear();
        } else {
            let parser_confirmed_ids = conversation
                .confirmed_queued_prompt_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            let queued_prompts = (loaders.queued_prompts)(&session_id)?;
            conversation.confirmed_queued_prompt_ids = pending_prompt_ids
                .into_iter()
                .filter(|message_id| {
                    parser_confirmed_ids.contains(message_id.as_str())
                        || queued_prompts.iter().any(|prompt| {
                            prompt.resolved
                                && prompt.message_id.as_deref() == Some(message_id.as_str())
                        })
                })
                .collect();
        }
        Ok(Some((conversation, vibe_mode)))
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
