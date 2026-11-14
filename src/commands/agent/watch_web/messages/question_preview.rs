use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::commands::agent::store::PermissionRequestData;
use crate::commands::agent::types::Session;

use super::cache::{TranscriptFingerprint, transcript_fingerprint};
use super::questions::question_item_from_input;
use super::{ChatItem, QuestionPrompt, RouteLoaders, TranscriptInput, incremental, parser};

const MAX_CACHED_QUESTION_PREVIEWS: usize = 256;

type QuestionPreviewCacheKey = (String, Option<TranscriptFingerprint>);
type QuestionPreviewCacheEntries = HashMap<QuestionPreviewCacheKey, Option<String>>;

#[derive(Clone, Default)]
pub(super) struct QuestionPreviewCache {
    entries: Arc<Mutex<QuestionPreviewCacheEntries>>,
}

impl QuestionPreviewCache {
    fn get(
        &self,
        session_id: &str,
        fingerprint: &Option<TranscriptFingerprint>,
    ) -> Option<Option<String>> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries
            .get(&(session_id.to_owned(), fingerprint.clone()))
            .cloned()
    }

    fn insert(
        &self,
        session_id: &str,
        fingerprint: Option<TranscriptFingerprint>,
        preview: Option<String>,
    ) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.retain(|(cached_session_id, _), _| cached_session_id != session_id);
        if entries.len() >= MAX_CACHED_QUESTION_PREVIEWS
            && let Some(key) = entries.keys().next().cloned()
        {
            entries.remove(&key);
        }
        entries.insert((session_id.to_owned(), fingerprint), preview);
    }
}

impl RouteLoaders {
    pub(in crate::commands::agent::watch_web) fn question_preview(
        &self,
        session: &Session,
    ) -> Result<Option<String>> {
        if let Some(preview) = transcript_question_preview(session, &self.question_preview_cache) {
            return Ok(Some(preview));
        }
        for agent_id in &session.pending_permission_agent_ids {
            let Some(request_id) = session.pending_permission_request_ids.get(agent_id) else {
                continue;
            };
            let Some(data) = (self.permission_request_data)(&session.session_id, request_id)?
            else {
                continue;
            };
            if let Some(preview) = permission_question_preview(request_id, &data) {
                return Ok(Some(preview));
            }
        }
        Ok(None)
    }
}

fn transcript_question_preview(session: &Session, cache: &QuestionPreviewCache) -> Option<String> {
    let fingerprint =
        match transcript_fingerprint(session.engine, session.transcript_path.as_deref()) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                tracing::warn!(
                    session_id = %session.session_id,
                    %error,
                    "failed to fingerprint session question preview transcript",
                );
                return None;
            }
        };
    if let Some(preview) = cache.get(&session.session_id, &fingerprint) {
        return preview;
    }

    let input = TranscriptInput {
        path: session.transcript_path.as_deref(),
        engine: session.engine,
        additions: parser::claude::ParserAdditions {
            session_id: &session.session_id,
            parent: None,
            pending_tool: None,
            user_marker: None,
            decisions: &[],
            queued_prompts: &[],
            peer_out_messages: &[],
            proposals: &[],
            todos: &[],
        },
    };
    match incremental::parse_session_transcript(input) {
        Ok((conversation, _)) => {
            let questions = conversation.messages.iter().rev().find_map(|message| {
                let ChatItem::Question { questions, .. } = message else {
                    return None;
                };
                Some(questions)
            });
            let preview = questions.and_then(|questions| {
                if questions.iter().any(|question| question.answer.is_none()) {
                    format_question_preview(questions)
                } else {
                    None
                }
            });
            cache.insert(&session.session_id, fingerprint, preview.clone());
            preview
        }
        Err(error) => {
            tracing::warn!(
                session_id = %session.session_id,
                %error,
                "failed to parse session question preview transcript",
            );
            None
        }
    }
}

fn permission_question_preview(request_id: &str, data: &PermissionRequestData) -> Option<String> {
    if data.tool_name.as_deref() != Some("AskUserQuestion") {
        return None;
    }
    let Some(ChatItem::Question { questions, .. }) =
        question_item_from_input(Some(request_id.to_owned()), &data.tool_input)
    else {
        return None;
    };
    format_question_preview(&questions)
}

fn format_question_preview(questions: &[QuestionPrompt]) -> Option<String> {
    let first_question = questions.first()?;
    Some(if questions.len() > 1 {
        format!("{} +{}", first_question.question, questions.len() - 1)
    } else {
        first_question.question.clone()
    })
}
