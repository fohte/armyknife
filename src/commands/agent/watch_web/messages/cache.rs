use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::commands::agent::store::{
    AgentDecision, AgentProposal, AgentTodo, PermissionDeniedOperation, WatchWebPeerOutMessage,
    WatchWebQueuedPrompt,
};
use crate::commands::agent::types::{Engine, Session};

use super::parser::{claude::IncrementalClaudeParser, codex::CodexParserState};
use super::{ChatItem, UserContentPart};
use super::{ConversationResponse, ParentSession, RouteLoaders};

mod transcript_fingerprint;

use transcript_fingerprint::transcript_fingerprint;
pub(super) use transcript_fingerprint::{TranscriptChange, TranscriptFingerprint};

const MAX_CACHED_SESSIONS: usize = 8;
const MAX_CACHED_BACKGROUND_TASK_SESSIONS: usize = 256;

#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct MessageCacheRecords {
    pub(super) proposals: Vec<AgentProposal>,
    pub(super) todos: Vec<AgentTodo>,
    pub(super) permission_denials: Vec<PermissionDeniedOperation>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct MessageCacheKey {
    pub(super) engine: Engine,
    pub(super) transcript_path: Option<PathBuf>,
    pub(super) transcript_fingerprint: Option<TranscriptFingerprint>,
    pub(super) parent: Option<ParentSession>,
    pub(super) pending_permission_agent_ids: BTreeSet<String>,
    pub(super) pending_permission_request_ids: BTreeMap<String, String>,
    pub(super) current_tool: Option<String>,
    pub(super) user_marker: Option<String>,
    pub(super) decisions: Vec<AgentDecision>,
    pub(super) queued_prompts: Vec<WatchWebQueuedPrompt>,
    pub(super) peer_out_messages: Vec<WatchWebPeerOutMessage>,
    pub(super) proposals: Vec<AgentProposal>,
    pub(super) todos: Vec<AgentTodo>,
    pub(super) permission_denials: Vec<PermissionDeniedOperation>,
}

impl MessageCacheKey {
    pub(super) fn new(
        session: &Session,
        parent: Option<ParentSession>,
        user_marker: Option<String>,
        decisions: Vec<AgentDecision>,
        queued_prompts: Vec<WatchWebQueuedPrompt>,
        peer_out_messages: Vec<WatchWebPeerOutMessage>,
        records: MessageCacheRecords,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            engine: session.engine,
            transcript_path: session.transcript_path.clone(),
            transcript_fingerprint: transcript_fingerprint(
                session.engine,
                session.transcript_path.as_deref(),
            )?,
            parent,
            pending_permission_agent_ids: session.pending_permission_agent_ids.clone(),
            pending_permission_request_ids: session.pending_permission_request_ids.clone(),
            current_tool: session.current_tool.clone(),
            user_marker,
            decisions,
            queued_prompts,
            peer_out_messages,
            proposals: records.proposals,
            todos: records.todos,
            permission_denials: records.permission_denials,
        })
    }
}

#[derive(Clone, Default)]
pub(super) struct RunningBackgroundTaskCache {
    entries: Arc<Mutex<RunningBackgroundTaskEntries>>,
}

type RunningBackgroundTaskEntries = HashMap<String, (Option<TranscriptFingerprint>, bool)>;

impl RunningBackgroundTaskCache {
    pub(super) fn fingerprint(session: &Session) -> anyhow::Result<Option<TranscriptFingerprint>> {
        transcript_fingerprint(session.engine, session.transcript_path.as_deref())
    }

    pub(super) fn get(
        &self,
        session_id: &str,
        fingerprint: &Option<TranscriptFingerprint>,
    ) -> Option<bool> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (cached_fingerprint, has_running_task) = entries.get(session_id)?;
        (cached_fingerprint == fingerprint).then_some(*has_running_task)
    }

    pub(super) fn insert(
        &self,
        session_id: &str,
        fingerprint: Option<TranscriptFingerprint>,
        has_running_task: bool,
    ) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        insert_with_eviction(
            &mut entries,
            session_id.to_string(),
            (fingerprint, has_running_task),
            MAX_CACHED_BACKGROUND_TASK_SESSIONS,
        );
    }
}

pub(super) fn delete_resolved_queued_prompts(
    loaders: &RouteLoaders,
    session_id: &str,
    ids: &[i64],
) {
    if !ids.is_empty()
        && let Err(error) = (loaders.delete_queued_prompts)(session_id, ids)
    {
        tracing::warn!(
            session_id,
            %error,
            "failed to remove resolved watch-web queued prompts"
        );
    }
}

pub(super) fn refresh_image_availability(conversation: &mut ConversationResponse) {
    refresh_image_availability_with(
        conversation,
        super::super::image_attachments::image_is_available,
    );
}

fn refresh_image_availability_with(
    conversation: &mut ConversationResponse,
    mut is_available: impl FnMut(&str) -> bool,
) {
    for message in &mut conversation.messages {
        let content = match message {
            ChatItem::QueuedUser { content, .. }
            | ChatItem::UserWithContent { content, .. }
            | ChatItem::Peer { content, .. } => content,
            _ => continue,
        };
        for part in content {
            if let UserContentPart::Image { source, available } = part
                && source.starts_with("/api/images/")
            {
                *available = is_available(source);
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct CachedConversation {
    pub(super) conversation: ConversationResponse,
    pub(super) resolved_queued_prompt_ids: Vec<i64>,
    pub(super) cached_at: Instant,
}

#[derive(Clone, Default)]
pub(super) struct MessageCache {
    entries: Arc<Mutex<HashMap<String, (MessageCacheKey, CachedConversation)>>>,
    transcript_parsers: TranscriptParserCache,
}

impl MessageCache {
    pub(super) fn transcript_parser_slot(
        &self,
        session_id: &str,
    ) -> Arc<Mutex<Option<CachedTranscriptParser>>> {
        self.transcript_parsers.get_or_create_slot(session_id)
    }

    pub(super) fn get(
        &self,
        session_id: &str,
        key: &MessageCacheKey,
    ) -> anyhow::Result<Option<CachedConversation>> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some((cached_key, cached)) = entries.get(session_id) else {
            return Ok(None);
        };
        if cached_key != key {
            return Ok(None);
        }
        let cached = cached.clone();
        drop(entries);
        if transcript_fingerprint(key.engine, key.transcript_path.as_deref())?
            != key.transcript_fingerprint
        {
            return Ok(None);
        }
        Ok(Some(cached))
    }

    pub(super) fn insert(
        &self,
        session_id: &str,
        key: MessageCacheKey,
        conversation: ConversationResponse,
        resolved_queued_prompt_ids: Vec<i64>,
    ) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        insert_with_eviction(
            &mut entries,
            session_id.to_string(),
            (
                key,
                CachedConversation {
                    conversation,
                    resolved_queued_prompt_ids,
                    cached_at: Instant::now(),
                },
            ),
            MAX_CACHED_SESSIONS,
        );
    }
}

#[derive(Clone, Default)]
pub(super) struct TranscriptParserCache {
    entries: Arc<Mutex<TranscriptParserEntries>>,
}

type TranscriptParserSlot = Arc<Mutex<Option<CachedTranscriptParser>>>;
type TranscriptParserEntries = HashMap<String, TranscriptParserSlot>;

pub(super) struct CachedTranscriptParser {
    pub(super) fingerprint: TranscriptFingerprint,
    pub(super) tail: Vec<u8>,
    pub(super) engine: Engine,
    #[cfg(test)]
    pub(super) last_change: TranscriptChange,
    pub(super) parser: IncrementalTranscriptParser,
}

pub(super) enum IncrementalTranscriptParser {
    Claude(Box<IncrementalClaudeParser>),
    Codex(CodexParserState),
}

impl TranscriptParserCache {
    pub(super) fn get_or_create_slot(&self, session_id: &str) -> TranscriptParserSlot {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let slot = entries
            .get(session_id)
            .cloned()
            .unwrap_or_else(|| Arc::new(Mutex::new(None)));
        insert_with_eviction(
            &mut entries,
            session_id.to_string(),
            Arc::clone(&slot),
            MAX_CACHED_SESSIONS,
        );
        slot
    }
}

fn insert_with_eviction<K, V>(entries: &mut HashMap<K, V>, key: K, value: V, capacity: usize)
where
    K: Clone + Eq + std::hash::Hash,
{
    if !entries.contains_key(&key)
        && entries.len() >= capacity
        && let Some(evicted_key) = entries.keys().next().cloned()
    {
        entries.remove(&evicted_key);
    }
    entries.insert(key, value);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        ConversationResponse, MessageCacheKey, MessageCacheRecords, refresh_image_availability_with,
    };
    use crate::commands::agent::types::{MAIN_THREAD_AGENT_KEY, Session};
    use crate::commands::agent::watch_web::messages::{ChatItem, UserContentPart};

    #[test]
    fn message_cache_key_ignores_last_assistant_message() {
        let mut session: Session = serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "status": "ended",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": "Earlier response.",
            "pending_permission_agent_ids": [MAIN_THREAD_AGENT_KEY],
            "current_tool": "Bash(echo fixture)",
        }))
        .unwrap();

        let first_key = MessageCacheKey::new(
            &session,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            MessageCacheRecords::default(),
        )
        .unwrap();
        session.last_message = Some("Later response.".to_string());
        let second_key = MessageCacheKey::new(
            &session,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            MessageCacheRecords::default(),
        )
        .unwrap();

        assert!(first_key == second_key);
    }

    #[test]
    fn message_cache_key_changes_when_a_permission_request_is_replaced() {
        let mut session: Session = serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "status": "waiting_input",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "pending_permission_agent_ids": [MAIN_THREAD_AGENT_KEY],
            "pending_permission_request_ids": {},
        }))
        .unwrap();
        session.pending_permission_request_ids.insert(
            MAIN_THREAD_AGENT_KEY.to_string(),
            "permission-request-first".to_string(),
        );
        let first_key = MessageCacheKey::new(
            &session,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            MessageCacheRecords::default(),
        )
        .unwrap();
        session.pending_permission_request_ids.insert(
            MAIN_THREAD_AGENT_KEY.to_string(),
            "permission-request-next".to_string(),
        );
        let next_key = MessageCacheKey::new(
            &session,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            MessageCacheRecords::default(),
        )
        .unwrap();

        assert!(first_key != next_key);
    }

    #[test]
    fn refreshes_external_image_status_but_keeps_inline_images_available() {
        let mut conversation = ConversationResponse {
            messages: vec![ChatItem::UserWithContent {
                content: vec![
                    UserContentPart::Image {
                        source: "/api/images/fixture-image.png".to_string(),
                        available: true,
                    },
                    UserContentPart::Image {
                        source: "data:image/png;base64,AAAA".to_string(),
                        available: true,
                    },
                ],
                timestamp: None,
                from: None,
            }],
            background_tasks: Vec::new(),
            ..ConversationResponse::default()
        };

        refresh_image_availability_with(&mut conversation, |_| false);

        assert_eq!(
            conversation,
            ConversationResponse {
                messages: vec![ChatItem::UserWithContent {
                    content: vec![
                        UserContentPart::Image {
                            source: "/api/images/fixture-image.png".to_string(),
                            available: false,
                        },
                        UserContentPart::Image {
                            source: "data:image/png;base64,AAAA".to_string(),
                            available: true,
                        },
                    ],
                    timestamp: None,
                    from: None,
                }],
                background_tasks: Vec::new(),
                ..ConversationResponse::default()
            },
        );
    }
}
