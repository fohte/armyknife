use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Context;

use crate::commands::agent::store::{
    AgentDecision, AgentProposal, WatchWebPeerOutMessage, WatchWebQueuedPrompt,
};
use crate::commands::agent::types::{Engine, Session};

use super::{ChatItem, UserContentPart};
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
            transcript_fingerprint: transcript_fingerprint(
                session.engine,
                session.transcript_path.as_deref(),
            )?,
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

#[derive(Clone, PartialEq, Eq)]
pub(super) struct TranscriptFingerprint {
    source_path: PathBuf,
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

fn transcript_fingerprint(
    engine: Engine,
    path: Option<&Path>,
) -> anyhow::Result<Option<TranscriptFingerprint>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let source_path = if engine == Engine::Codex {
        match std::fs::metadata(path) {
            Ok(_) => path.to_path_buf(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let compressed_path = super::compressed_path(path);
                match std::fs::metadata(&compressed_path) {
                    Ok(_) => compressed_path,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(None);
                    }
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("read transcript metadata {}", compressed_path.display())
                        });
                    }
                }
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read transcript metadata {}", path.display()));
            }
        }
    } else {
        path.to_path_buf()
    };
    let metadata = match std::fs::metadata(&source_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read transcript metadata {}", source_path.display()));
        }
    };
    let modified = metadata.modified().with_context(|| {
        format!(
            "read transcript modification time {}",
            source_path.display()
        )
    })?;
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
        source_path,
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
}

#[cfg(test)]
mod tests {
    use super::{ConversationResponse, refresh_image_availability_with};
    use crate::commands::agent::watch_web::messages::{ChatItem, UserContentPart};

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
                from: None,
            }],
            background_tasks: Vec::new(),
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
                    from: None,
                }],
                background_tasks: Vec::new(),
            },
        );
    }
}
