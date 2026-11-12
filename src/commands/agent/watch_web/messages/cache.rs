use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Context;

use crate::commands::agent::store::{
    AgentDecision, AgentProposal, AgentTodo, WatchWebPeerOutMessage, WatchWebQueuedPrompt,
};
use crate::commands::agent::types::{Engine, Session};

use super::parser::{claude::IncrementalClaudeParser, codex::CodexParserState};
use super::{ChatItem, UserContentPart};
use super::{ConversationResponse, ParentSession, RouteLoaders};

const MAX_CACHED_SESSIONS: usize = 8;
const MAX_CACHED_BACKGROUND_TASK_SESSIONS: usize = 256;

#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct MessageCacheRecords {
    pub(super) proposals: Vec<AgentProposal>,
    pub(super) todos: Vec<AgentTodo>,
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
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TranscriptChange {
    Unchanged,
    Appended,
    Rewritten,
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

impl TranscriptFingerprint {
    pub(super) fn change_from(&self, previous: &Self) -> TranscriptChange {
        if self == previous {
            return TranscriptChange::Unchanged;
        }
        if self.source_path == previous.source_path
            && self.len > previous.len
            && self.has_same_file_identity(previous)
        {
            return TranscriptChange::Appended;
        }
        TranscriptChange::Rewritten
    }

    fn has_same_file_identity(&self, previous: &Self) -> bool {
        #[cfg(unix)]
        {
            self.device == previous.device && self.inode == previous.inode
        }
        #[cfg(not(unix))]
        {
            self.modified >= previous.modified
        }
    }

    pub(super) fn is_compressed(&self) -> bool {
        self.source_path
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("zst")
    }

    pub(super) fn read_delta_from(
        &self,
        previous: &Self,
    ) -> anyhow::Result<(TranscriptChange, Vec<u8>)> {
        let change = self.change_from(previous);
        match change {
            TranscriptChange::Unchanged => Ok((change, Vec::new())),
            TranscriptChange::Rewritten => Ok((change, Vec::new())),
            TranscriptChange::Appended => {
                if self
                    .source_path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("zst")
                {
                    return Ok((TranscriptChange::Rewritten, Vec::new()));
                }
                let mut file = File::open(&self.source_path)
                    .with_context(|| format!("open transcript {}", self.source_path.display()))?;
                file.seek(SeekFrom::Start(previous.len))
                    .with_context(|| format!("seek transcript {}", self.source_path.display()))?;
                let mut appended = Vec::new();
                file.take(self.len - previous.len)
                    .read_to_end(&mut appended)
                    .with_context(|| format!("read transcript {}", self.source_path.display()))?;
                if appended.len() as u64 != self.len - previous.len {
                    return Ok((TranscriptChange::Rewritten, Vec::new()));
                }
                Ok((change, appended))
            }
        }
    }
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
        if !entries.contains_key(session_id)
            && entries.len() >= MAX_CACHED_BACKGROUND_TASK_SESSIONS
            && let Some(evicted_session_id) = entries.keys().next().cloned()
        {
            entries.remove(&evicted_session_id);
        }
        entries.insert(session_id.to_string(), (fingerprint, has_running_task));
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

#[derive(Clone, Default)]
pub(super) struct TranscriptParserCache {
    entries: Arc<Mutex<TranscriptParserEntries>>,
}

type TranscriptParserSlot = Arc<Mutex<Option<CachedTranscriptParser>>>;
type TranscriptParserEntries = HashMap<String, TranscriptParserSlot>;

pub(super) struct CachedTranscriptParser {
    pub(super) fingerprint: TranscriptFingerprint,
    pub(super) engine: Engine,
    pub(super) parent_session_id: Option<String>,
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
        if !entries.contains_key(session_id)
            && entries.len() >= MAX_CACHED_SESSIONS
            && let Some(evicted_session_id) = entries.keys().next().cloned()
        {
            entries.remove(&evicted_session_id);
        }
        Arc::clone(
            entries
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(None))),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use serde_json::json;

    use super::{
        ConversationResponse, MessageCacheKey, MessageCacheRecords, TranscriptChange,
        refresh_image_availability_with, transcript_fingerprint,
    };
    use crate::commands::agent::types::{Engine, MAIN_THREAD_AGENT_KEY, Session};
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

    #[test]
    fn transcript_append_reads_only_bytes_after_the_cached_length() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("transcript.jsonl");
        std::fs::write(&path, b"first record\n").expect("transcript should be written");
        let previous = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("transcript should open for appending")
            .write_all(b"second record\n")
            .expect("record should be appended");
        let current = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");

        assert_eq!(
            current
                .read_delta_from(&previous)
                .expect("delta should be read"),
            (TranscriptChange::Appended, b"second record\n".to_vec()),
        );
    }

    #[test]
    fn transcript_rewrite_requires_a_full_read() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("transcript.jsonl");
        std::fs::write(&path, b"first record\n").expect("transcript should be written");
        let previous = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");
        std::fs::write(&path, b"other record\n").expect("transcript should be rewritten");
        let current = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");

        assert_eq!(
            current
                .read_delta_from(&previous)
                .expect("delta should be read"),
            (TranscriptChange::Rewritten, Vec::new()),
        );
    }
}
