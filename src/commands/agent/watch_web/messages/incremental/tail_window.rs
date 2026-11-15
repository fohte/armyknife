use anyhow::Result;

use super::super::cache::{IncrementalTranscriptParser, MessageCache, TranscriptFingerprint};
use super::super::parser;
use super::super::parser::cards::filter_additional_cards_after_tail_window;
use super::super::{
    ChatItem, ConversationResponse, Engine, TranscriptInput, open_seekable_transcript,
};
use super::render_parser;

pub(in crate::commands::agent::watch_web::messages) fn parse_initial_or_full<'a>(
    session_id: &str,
    input: TranscriptInput<'a>,
    tail_window_limit: Option<usize>,
    pending_task_ids: &[String],
    fingerprint: Option<TranscriptFingerprint>,
    cache: &MessageCache,
) -> Result<(ConversationResponse, Vec<i64>, bool)> {
    let tail_result = match tail_window_limit {
        Some(limit) => parse_session_transcript_tail_window(&input, limit, pending_task_ids)?,
        None => None,
    };
    match tail_result {
        Some((conversation, resolved_ids)) => Ok((conversation, resolved_ids, true)),
        None => {
            super::parse_session_transcript_incrementally(session_id, input, fingerprint, cache)
                .map(|(conversation, resolved_ids)| (conversation, resolved_ids, false))
        }
    }
}

pub(in crate::commands::agent::watch_web::messages) fn filter_after_tail_window<T: Clone>(
    records: &[T],
    cutoff: Option<chrono::DateTime<chrono::Utc>>,
    created_at: impl Fn(&T) -> chrono::DateTime<chrono::Utc>,
) -> Vec<T> {
    records
        .iter()
        .filter(|record| {
            cutoff
                .as_ref()
                .is_none_or(|cutoff| created_at(record) >= *cutoff)
        })
        .cloned()
        .collect()
}

pub(in crate::commands::agent::watch_web::messages) fn parse_session_transcript_tail_window(
    input: &TranscriptInput<'_>,
    message_limit: usize,
    pending_task_ids: &[String],
) -> Result<Option<(ConversationResponse, Vec<i64>)>> {
    if input.engine != Engine::Claude
        || input
            .path
            .and_then(|path| path.extension())
            .and_then(|extension| extension.to_str())
            == Some("zst")
    {
        return Ok(None);
    }
    let reader = input
        .path
        .map(open_seekable_transcript)
        .transpose()?
        .flatten()
        .unwrap_or_else(|| Box::new(std::io::Cursor::new(Vec::new())));
    let image_directory = super::super::super::image_attachments::storage_directory().ok();
    let Some(parser) = parser::claude::IncrementalClaudeParser::new_tail_window(
        reader,
        image_directory.as_deref(),
        message_limit,
        pending_task_ids,
    )?
    else {
        return Ok(None);
    };
    let Some(cutoff) = parser.tail_window_start() else {
        return Ok(None);
    };
    let Some((queued_prompts, prior_resolved_ids)) = resolve_queued_prompts_before_window(
        input.path,
        parser.tail_window_start_offset().unwrap_or_default(),
        input.additions.queued_prompts,
        cutoff,
        input.additions.user_marker,
        &parser,
    )?
    else {
        return Ok(None);
    };
    let decisions = input
        .additions
        .decisions
        .iter()
        .filter(|decision| {
            decision.review.is_none()
                || chrono::DateTime::parse_from_rfc3339(&decision.created_at)
                    .is_ok_and(|created_at| created_at.with_timezone(&chrono::Utc) >= cutoff)
        })
        .cloned()
        .collect::<Vec<_>>();
    let proposals = input
        .additions
        .proposals
        .iter()
        .filter(|proposal| {
            matches!(
                &proposal.item,
                ChatItem::Proposal {
                    resolved: false,
                    ..
                }
            ) || proposal.created_at >= cutoff
        })
        .cloned()
        .collect::<Vec<_>>();
    let todos = input
        .additions
        .todos
        .iter()
        .filter(|todo| todo.completed_at.is_none() || todo.created_at >= cutoff)
        .cloned()
        .collect::<Vec<_>>();
    let peer_out_messages = input
        .additions
        .peer_out_messages
        .iter()
        .filter(|message| message.sent_at >= cutoff)
        .cloned()
        .collect::<Vec<_>>();
    let additional_cards =
        filter_additional_cards_after_tail_window(input.additions.additional_cards, cutoff);
    let tail_input = TranscriptInput {
        path: input.path,
        engine: input.engine,
        additions: parser::claude::ParserAdditions {
            decisions: &decisions,
            proposals: &proposals,
            todos: &todos,
            peer_out_messages: &peer_out_messages,
            additional_cards: &additional_cards,
            queued_prompts: &queued_prompts,
            ..input.additions
        },
    };
    let (mut conversation, mut resolved_ids) = render_parser(
        &IncrementalTranscriptParser::Claude(Box::new(parser)),
        &tail_input,
    )?;
    // Branch filtering can leave older visible messages outside the selected suffix.
    if conversation.messages.len() < message_limit {
        return Ok(None);
    }
    resolved_ids.extend(prior_resolved_ids);
    resolved_ids.sort_unstable();
    resolved_ids.dedup();
    conversation.tail_window_start = Some(cutoff);
    Ok(Some((conversation, resolved_ids)))
}

fn resolve_queued_prompts_before_window(
    path: Option<&std::path::Path>,
    window_start: u64,
    queued_prompts: &[crate::commands::agent::store::WatchWebQueuedPrompt],
    cutoff: chrono::DateTime<chrono::Utc>,
    user_marker: Option<&str>,
    parser: &parser::claude::IncrementalClaudeParser,
) -> Result<
    Option<(
        Vec<crate::commands::agent::store::WatchWebQueuedPrompt>,
        Vec<i64>,
    )>,
> {
    let Some(path) = path else {
        return Ok(Some((queued_prompts.to_vec(), Vec::new())));
    };
    let unresolved_prompts = queued_prompts
        .iter()
        .filter(|prompt| !prompt.resolved && prompt.sent_at < cutoff)
        .cloned()
        .collect::<Vec<_>>();
    if unresolved_prompts.is_empty() {
        return Ok(Some((queued_prompts.to_vec(), Vec::new())));
    }
    let Some(mut reader) = open_seekable_transcript(path)? else {
        return Ok(Some((queued_prompts.to_vec(), Vec::new())));
    };
    let Some(prior_resolved_ids) = parser.resolve_queued_prompt_ids_before_window(
        &mut reader,
        window_start,
        &unresolved_prompts,
        user_marker,
    )?
    else {
        return Ok(None);
    };
    let resolved_set = prior_resolved_ids
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let remaining_prompts = queued_prompts
        .iter()
        .filter(|prompt| !resolved_set.contains(&prompt.id))
        .cloned()
        .collect();
    Ok(Some((remaining_prompts, prior_resolved_ids)))
}
