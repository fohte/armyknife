use std::collections::HashSet;

use super::cache::RunningBackgroundTaskCache;
use super::parser::claude::ParserAdditions;
use super::{TranscriptInput, parse_session_transcript};
use crate::commands::agent::types::{Engine, Session};

pub(super) fn running_background_task_session_ids(
    sessions: &[Session],
    cache: &RunningBackgroundTaskCache,
) -> HashSet<String> {
    let mut session_ids = HashSet::new();
    let decisions = [];
    let queued_prompts = [];
    let peer_out_messages = [];
    let proposals = [];

    for session in sessions
        .iter()
        .filter(|session| session.engine == Engine::Claude)
    {
        let fingerprint = match RunningBackgroundTaskCache::fingerprint(session) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                tracing::warn!(
                    event = "agent.watch_web.background_task_fingerprint_failed",
                    session_id = %session.session_id,
                    %error,
                );
                if session.has_pending_bg_tasks() {
                    session_ids.insert(session.session_id.clone());
                }
                continue;
            }
        };
        let has_running_task = cache.get(&session.session_id, &fingerprint).or_else(|| {
            if fingerprint.is_none() {
                return Some(session.has_pending_bg_tasks());
            }

            match parse_session_transcript(TranscriptInput {
                path: session.transcript_path.as_deref(),
                engine: Engine::Claude,
                parent: None,
                pending_tool: None,
                watch_web_user_marker: None,
                additions: ParserAdditions {
                    decisions: &decisions,
                    queued_prompts: &queued_prompts,
                    peer_out_messages: &peer_out_messages,
                    proposals: &proposals,
                },
            }) {
                Ok((conversation, _)) => {
                    let has_running_task = !conversation.background_tasks.is_empty();
                    cache.insert(&session.session_id, fingerprint.clone(), has_running_task);
                    Some(has_running_task)
                }
                Err(error) => {
                    tracing::warn!(
                        event = "agent.watch_web.background_task_transcript_parse_failed",
                        session_id = %session.session_id,
                        %error,
                    );
                    Some(session.has_pending_bg_tasks())
                }
            }
        });

        if has_running_task.unwrap_or_else(|| session.has_pending_bg_tasks()) {
            session_ids.insert(session.session_id.clone());
        }
    }

    session_ids
}
