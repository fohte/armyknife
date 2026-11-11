use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Context;

use crate::commands::agent::store::{
    AgentDecision, AgentProposal, WatchWebPeerOutMessage, WatchWebQueuedPrompt,
};
use crate::commands::agent::types::{Engine, Session};

use super::{ConversationResponse, ParentSession, RouteLoaders};

const MAX_CACHED_SESSIONS: usize = 8;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct MessageCacheKey {
    pub(super) engine: Engine,
    pub(super) transcript_path: Option<PathBuf>,
    pub(super) transcript_fingerprint: Option<TranscriptFingerprint>,
    pub(super) parent: Option<ParentSession>,
    pub(super) pending_permission_agent_ids: BTreeSet<String>,
    pub(super) current_tool: Option<String>,
    pub(super) last_message: Option<String>,
    pub(super) user_marker: Option<String>,
    pub(super) decisions: Vec<AgentDecision>,
    pub(super) queued_prompts: Vec<WatchWebQueuedPrompt>,
    pub(super) peer_out_messages: Vec<WatchWebPeerOutMessage>,
    pub(super) proposals: Vec<AgentProposal>,
}

impl MessageCacheKey {
    pub(super) fn new(
        session: &Session,
        parent: Option<ParentSession>,
        user_marker: Option<String>,
        decisions: Vec<AgentDecision>,
        queued_prompts: Vec<WatchWebQueuedPrompt>,
        peer_out_messages: Vec<WatchWebPeerOutMessage>,
        proposals: Vec<AgentProposal>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            engine: session.engine,
            transcript_path: session.transcript_path.clone(),
            transcript_fingerprint: transcript_fingerprint(session.transcript_path.as_deref())?,
            parent,
            pending_permission_agent_ids: session.pending_permission_agent_ids.clone(),
            current_tool: session.current_tool.clone(),
            last_message: session.last_message.clone(),
            user_marker,
            decisions,
            queued_prompts,
            peer_out_messages,
            proposals,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct TranscriptFingerprint {
    len: u64,
    modified: std::time::SystemTime,
    // ctime catches same-size rewrites even if the modification time is restored.
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed_at_seconds: i64,
    #[cfg(unix)]
    changed_at_nanoseconds: i64,
}

fn transcript_fingerprint(path: Option<&Path>) -> anyhow::Result<Option<TranscriptFingerprint>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read transcript metadata {}", path.display()));
        }
    };
    let modified = metadata
        .modified()
        .with_context(|| format!("read transcript modification time {}", path.display()))?;
    #[cfg(unix)]
    let (device, inode, changed_at_seconds, changed_at_nanoseconds) = {
        use std::os::unix::fs::MetadataExt;

        (
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        )
    };
    Ok(Some(TranscriptFingerprint {
        len: metadata.len(),
        modified,
        #[cfg(unix)]
        device,
        #[cfg(unix)]
        inode,
        #[cfg(unix)]
        changed_at_seconds,
        #[cfg(unix)]
        changed_at_nanoseconds,
    }))
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

#[derive(Clone)]
pub(super) struct CachedConversation {
    pub(super) conversation: ConversationResponse,
    pub(super) resolved_queued_prompt_ids: Vec<i64>,
    pub(super) cached_at: Instant,
}

#[derive(Clone, Default)]
pub(super) struct MessageCache {
    entries: Arc<Mutex<HashMap<String, (MessageCacheKey, CachedConversation)>>>,
    #[cfg(test)]
    hits: Arc<AtomicUsize>,
}

impl MessageCache {
    pub(super) fn get(
        &self,
        session_id: &str,
        key: &MessageCacheKey,
    ) -> Option<CachedConversation> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (cached_key, cached) = entries.get(session_id)?;
        if cached_key != key {
            return None;
        }
        #[cfg(test)]
        self.hits.fetch_add(1, Ordering::Relaxed);
        Some(cached.clone())
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
        if !entries.contains_key(session_id)
            && entries.len() >= MAX_CACHED_SESSIONS
            && let Some(evicted_session_id) = entries.keys().next().cloned()
        {
            entries.remove(&evicted_session_id);
        }
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

    #[cfg(test)]
    pub(super) fn hit_count(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }
}
