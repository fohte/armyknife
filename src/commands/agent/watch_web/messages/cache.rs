use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::commands::agent::store::{
    AgentDecision, AgentFile, AgentProposal, AgentTodo, PermissionDeniedOperation,
    WatchWebPeerOutMessage, WatchWebQueuedPrompt,
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
    pub(super) agent_files: Vec<AgentFile>,
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
    pub(super) current_tool_name: Option<String>,
    pub(super) user_marker: Option<String>,
    pub(super) decisions: Vec<AgentDecision>,
    pub(super) queued_prompts: Vec<WatchWebQueuedPrompt>,
    pub(super) peer_out_messages: Vec<WatchWebPeerOutMessage>,
    pub(super) proposals: Vec<AgentProposal>,
    pub(super) todos: Vec<AgentTodo>,
    pub(super) agent_files: Vec<AgentFile>,
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
            current_tool_name: session.current_tool_name.clone(),
            user_marker,
            decisions,
            queued_prompts,
            peer_out_messages,
            proposals: records.proposals,
            todos: records.todos,
            agent_files: records.agent_files,
            permission_denials: records.permission_denials,
        })
    }
}

#[derive(Clone)]
pub(super) struct RunningBackgroundTaskCache {
    entries: Arc<Mutex<RunningBackgroundTaskEntries>>,
}

type RunningBackgroundTaskEntries = LruCache<String, (Option<TranscriptFingerprint>, bool)>;

impl Default for RunningBackgroundTaskCache {
    fn default() -> Self {
        Self {
            entries: Arc::new(Mutex::new(LruCache::new(
                MAX_CACHED_BACKGROUND_TASK_SESSIONS,
            ))),
        }
    }
}

impl RunningBackgroundTaskCache {
    pub(super) fn fingerprint(session: &Session) -> anyhow::Result<Option<TranscriptFingerprint>> {
        transcript_fingerprint(session.engine, session.transcript_path.as_deref())
    }

    pub(super) fn get(
        &self,
        session_id: &str,
        fingerprint: &Option<TranscriptFingerprint>,
    ) -> Option<bool> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (cached_fingerprint, has_running_task) = entries.get(&session_id.to_string())?;
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
        entries.insert(session_id.to_string(), (fingerprint, has_running_task));
    }
}

pub(super) fn mark_resolved_queued_prompts(loaders: &RouteLoaders, session_id: &str, ids: &[i64]) {
    if !ids.is_empty()
        && let Err(error) = (loaders.mark_queued_prompts_resolved)(session_id, ids)
    {
        tracing::warn!(
            session_id,
            %error,
            "failed to mark resolved watch-web queued prompts"
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

#[derive(Clone)]
pub(super) struct MessageCache {
    entries: Arc<Mutex<LruCache<String, (MessageCacheKey, CachedConversation)>>>,
    transcript_parsers: TranscriptParserCache,
}

impl Default for MessageCache {
    fn default() -> Self {
        Self {
            entries: Arc::new(Mutex::new(LruCache::new(MAX_CACHED_SESSIONS))),
            transcript_parsers: TranscriptParserCache::default(),
        }
    }
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
        let cached = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some((cached_key, cached)) = entries.get(&session_id.to_string()) else {
                return Ok(None);
            };
            if cached_key != key {
                return Ok(None);
            }
            cached.clone()
        };
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
        entries.insert(
            session_id.to_string(),
            (
                key,
                CachedConversation {
                    conversation,
                    resolved_queued_prompt_ids,
                    cached_at: Instant::now(),
                },
            ),
        );
    }
}

#[derive(Clone)]
pub(super) struct TranscriptParserCache {
    entries: Arc<Mutex<LruCache<String, TranscriptParserSlot>>>,
}

type TranscriptParserSlot = Arc<Mutex<Option<CachedTranscriptParser>>>;

impl Default for TranscriptParserCache {
    fn default() -> Self {
        Self {
            entries: Arc::new(Mutex::new(LruCache::new(MAX_CACHED_SESSIONS))),
        }
    }
}

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
        entries
            .get(&session_id.to_string())
            .cloned()
            .unwrap_or_else(|| {
                let slot = Arc::new(Mutex::new(None));
                entries.insert(session_id.to_string(), Arc::clone(&slot));
                slot
            })
    }
}

struct LruEntry<V> {
    value: V,
    last_used: u64,
}

struct LruCache<K, V> {
    entries: HashMap<K, LruEntry<V>>,
    access_counter: u64,
    capacity: usize,
}

impl<K, V> LruCache<K, V> {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            access_counter: 0,
            capacity,
        }
    }
}

impl<K, V> LruCache<K, V>
where
    K: Clone + Eq + std::hash::Hash,
{
    fn get(&mut self, key: &K) -> Option<&V> {
        let entry = self.entries.get_mut(key)?;
        self.access_counter += 1;
        entry.last_used = self.access_counter;
        Some(&entry.value)
    }

    fn insert(&mut self, key: K, value: V) {
        self.access_counter += 1;
        if !self.entries.contains_key(&key)
            && self.entries.len() >= self.capacity
            && let Some(evicted_key) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&evicted_key);
        }
        self.entries.insert(
            key,
            LruEntry {
                value,
                last_used: self.access_counter,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::sync::Arc;

    use super::{
        ConversationResponse, MAX_CACHED_BACKGROUND_TASK_SESSIONS, MAX_CACHED_SESSIONS,
        MessageCache, MessageCacheKey, MessageCacheRecords, RunningBackgroundTaskCache,
        TranscriptParserCache, refresh_image_availability_with,
    };
    use crate::commands::agent::types::{MAIN_THREAD_AGENT_KEY, Session};
    use crate::commands::agent::watch_web::messages::{ChatItem, UserContentPart};

    fn message_cache_key() -> MessageCacheKey {
        let session: Session = serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "status": "ended",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
        }))
        .unwrap();
        MessageCacheKey::new(
            &session,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            MessageCacheRecords::default(),
        )
        .unwrap()
    }

    #[test]
    fn message_cache_evicts_the_least_recently_used_session() {
        let cache = MessageCache::default();
        let key = message_cache_key();
        for index in 0..MAX_CACHED_SESSIONS {
            cache.insert(
                &format!("fixture-session-{index}"),
                key.clone(),
                ConversationResponse::default(),
                Vec::new(),
            );
        }

        let was_read = cache.get("fixture-session-0", &key).unwrap().is_some();
        cache.insert(
            "fixture-session-8",
            key.clone(),
            ConversationResponse::default(),
            Vec::new(),
        );
        cache.insert(
            "fixture-session-3",
            key.clone(),
            ConversationResponse::default(),
            Vec::new(),
        );
        cache.insert(
            "fixture-session-9",
            key.clone(),
            ConversationResponse::default(),
            Vec::new(),
        );

        let result = (
            was_read,
            [
                cache.get("fixture-session-0", &key).unwrap().is_some(),
                cache.get("fixture-session-3", &key).unwrap().is_some(),
            ],
            [
                cache.get("fixture-session-1", &key).unwrap().is_none(),
                cache.get("fixture-session-2", &key).unwrap().is_none(),
            ],
        );
        assert_eq!(result, (true, [true, true], [true, true]));
    }

    #[test]
    fn transcript_parser_cache_evicts_the_least_recently_used_session() {
        let cache = TranscriptParserCache::default();
        let slots = (0..MAX_CACHED_SESSIONS)
            .map(|index| cache.get_or_create_slot(&format!("fixture-session-{index}")))
            .collect::<Vec<_>>();

        let recently_used = cache.get_or_create_slot("fixture-session-0");
        cache.get_or_create_slot("fixture-session-8");
        let evicted = cache.get_or_create_slot("fixture-session-1");

        let result = (
            Arc::ptr_eq(&slots[0], &recently_used),
            Arc::ptr_eq(&slots[1], &evicted),
        );
        assert_eq!(result, (true, false));
    }

    #[test]
    fn running_background_task_cache_evicts_the_least_recently_used_session() {
        let cache = RunningBackgroundTaskCache::default();
        for index in 0..MAX_CACHED_BACKGROUND_TASK_SESSIONS {
            cache.insert(&format!("fixture-session-{index}"), None, false);
        }

        let was_read = cache.get("fixture-session-0", &None) == Some(false);
        cache.insert("fixture-session-256", None, false);
        cache.insert("fixture-session-3", None, true);
        cache.insert("fixture-session-257", None, false);

        let result = (
            was_read,
            [
                cache.get("fixture-session-0", &None),
                cache.get("fixture-session-3", &None),
            ],
            [
                cache.get("fixture-session-1", &None),
                cache.get("fixture-session-2", &None),
            ],
        );
        assert_eq!(result, (true, [Some(false), Some(true)], [None, None]));
    }

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
    fn message_cache_key_changes_when_the_structured_tool_name_changes() {
        let mut session: Session = serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "status": "waiting_input",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "current_tool": "FixtureTool(details)",
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
        session.current_tool_name = Some("FixtureTool".to_string());
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
                rewind_id: None,
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
                queued_prompt_id: None,
            }],
            background_tasks: Vec::new(),
            ..ConversationResponse::default()
        };

        refresh_image_availability_with(&mut conversation, |_| false);

        assert_eq!(
            conversation,
            ConversationResponse {
                messages: vec![ChatItem::UserWithContent {
                    rewind_id: None,
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
                    queued_prompt_id: None,
                }],
                background_tasks: Vec::new(),
                ..ConversationResponse::default()
            },
        );
    }
}
