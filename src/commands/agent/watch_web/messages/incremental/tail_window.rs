use anyhow::Result;

use super::super::cache::{IncrementalTranscriptParser, MessageCache, TranscriptFingerprint};
use super::super::parser;
use super::super::parser::cards::filter_additional_cards_after_tail_window;
use super::super::{
    ChatItem, ConversationResponse, Engine, TranscriptInput, open_seekable_transcript,
};
use super::render_parser;
use crate::commands::agent::store::{AgentDecision, AgentTodo};

pub(in crate::commands::agent::watch_web::messages) fn parse_initial_or_full<'a>(
    session_id: &str,
    input: TranscriptInput<'a>,
    tail_window_limit: Option<usize>,
    pending_task_ids: &[String],
    fingerprint: Option<TranscriptFingerprint>,
    cache: &MessageCache,
) -> Result<(ConversationResponse, Vec<i64>, bool)> {
    let tail_result = match tail_window_limit {
        Some(limit) => parse_session_transcript_tail_window(&input, limit, pending_task_ids),
        None => Ok(None),
    };
    parse_tail_window_or_full(tail_result, || {
        super::parse_session_transcript_incrementally(session_id, input, fingerprint, cache)
    })
    .map(|((conversation, resolved_ids), used_tail_window)| {
        (conversation, resolved_ids, used_tail_window)
    })
}

fn parse_tail_window_or_full<T>(
    tail_result: Result<Option<T>>,
    full_parse: impl FnOnce() -> Result<T>,
) -> Result<(T, bool)> {
    match tail_result {
        Ok(Some(tail_result)) => Ok((tail_result, true)),
        Ok(None) => full_parse().map(|result| (result, false)),
        Err(error) if contains_io_error(&error) => {
            tracing::warn!(
                error = %error,
                "failed to read transcript tail window; falling back to full parse"
            );
            full_parse().map(|result| (result, false))
        }
        Err(error) => Err(error),
    }
}

fn contains_io_error(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<std::io::Error>().is_some())
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
    parse_session_transcript_tail_window_with_max_bytes(
        input,
        message_limit,
        pending_task_ids,
        None,
    )
}

pub(in crate::commands::agent::watch_web::messages) fn parse_session_transcript_tail_window_bounded(
    input: &TranscriptInput<'_>,
    message_limit: usize,
    pending_task_ids: &[String],
    max_window_bytes: u64,
) -> Result<Option<(ConversationResponse, Vec<i64>)>> {
    parse_session_transcript_tail_window_with_max_bytes(
        input,
        message_limit,
        pending_task_ids,
        Some(max_window_bytes),
    )
}

fn parse_session_transcript_tail_window_with_max_bytes(
    input: &TranscriptInput<'_>,
    message_limit: usize,
    pending_task_ids: &[String],
    max_window_bytes: Option<u64>,
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
    let parser = match max_window_bytes {
        Some(max_window_bytes) => parser::claude::IncrementalClaudeParser::new_tail_window_bounded(
            reader,
            image_directory.as_deref(),
            message_limit,
            pending_task_ids,
            max_window_bytes,
        )?,
        None => parser::claude::IncrementalClaudeParser::new_tail_window(
            reader,
            image_directory.as_deref(),
            message_limit,
            pending_task_ids,
        )?,
    };
    let Some(parser) = parser else {
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
    let decisions = filter_decisions_after_tail_window(input.additions.decisions, cutoff);
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
    let todos = filter_todos_after_tail_window(input.additions.todos, cutoff);
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

fn filter_decisions_after_tail_window(
    decisions: &[AgentDecision],
    cutoff: chrono::DateTime<chrono::Utc>,
) -> Vec<AgentDecision> {
    decisions
        .iter()
        .filter(|decision| {
            chrono::DateTime::parse_from_rfc3339(&decision.created_at)
                .is_ok_and(|created_at| created_at.with_timezone(&chrono::Utc) >= cutoff)
        })
        .cloned()
        .collect()
}

fn filter_todos_after_tail_window(
    todos: &[AgentTodo],
    cutoff: chrono::DateTime<chrono::Utc>,
) -> Vec<AgentTodo> {
    todos
        .iter()
        .filter(|todo| todo.completed_at.is_none() || todo.created_at >= cutoff)
        .cloned()
        .collect()
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

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::{
        filter_decisions_after_tail_window, filter_todos_after_tail_window,
        parse_tail_window_or_full,
    };
    use crate::commands::agent::store::{AgentDecision, AgentTodo, TodoResolution};

    fn timestamp(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("test timestamp should parse")
            .with_timezone(&Utc)
    }

    #[test]
    fn tail_window_io_error_falls_back_to_full_parse() {
        let full_result = ("full transcript".to_owned(), vec![3, 7]);
        let actual = parse_tail_window_or_full::<(String, Vec<i64>)>(
            Err(std::io::Error::other("fixture read failure").into()),
            || Ok(full_result.clone()),
        )
        .unwrap();

        assert_eq!(actual, (full_result, false));
    }

    #[test]
    fn tail_window_omits_old_decisions_but_keeps_unresolved_todos() {
        let decisions = [
            AgentDecision {
                session_id: "fixture-session".to_string(),
                seq: 1,
                title: "An old unreviewed decision".to_string(),
                why: "It predates the displayed transcript window.".to_string(),
                considered: Vec::new(),
                easy_to_change: "Change it later.".to_string(),
                created_at: "2026-06-01T11:59:59Z".to_string(),
                review: None,
                reviewed_at: None,
            },
            AgentDecision {
                session_id: "fixture-session".to_string(),
                seq: 2,
                title: "A recent reviewed decision".to_string(),
                why: "It falls inside the displayed transcript window.".to_string(),
                considered: Vec::new(),
                easy_to_change: "Change it later.".to_string(),
                created_at: "2026-06-01T12:00:00Z".to_string(),
                review: Some("ok".to_string()),
                reviewed_at: Some("2026-06-01T12:00:01Z".to_string()),
            },
        ];
        let todos = [
            AgentTodo {
                session_id: "fixture-session".to_string(),
                seq: 1,
                title: "An old incomplete todo".to_string(),
                detail: None,
                created_at: timestamp("2026-06-01T11:59:59Z"),
                completed_at: None,
                resolution: None,
                comment: None,
            },
            AgentTodo {
                session_id: "fixture-session".to_string(),
                seq: 2,
                title: "An old completed todo".to_string(),
                detail: None,
                created_at: timestamp("2026-06-01T11:59:58Z"),
                completed_at: Some(timestamp("2026-06-01T11:59:59Z")),
                resolution: Some(TodoResolution::Completed),
                comment: None,
            },
            AgentTodo {
                session_id: "fixture-session".to_string(),
                seq: 3,
                title: "A recent completed todo".to_string(),
                detail: None,
                created_at: timestamp("2026-06-01T12:00:00Z"),
                completed_at: Some(timestamp("2026-06-01T12:00:01Z")),
                resolution: Some(TodoResolution::Completed),
                comment: None,
            },
        ];
        let cutoff = timestamp("2026-06-01T12:00:00Z");

        assert_eq!(
            (
                filter_decisions_after_tail_window(&decisions, cutoff),
                filter_todos_after_tail_window(&todos, cutoff),
            ),
            (
                vec![decisions[1].clone()],
                vec![todos[0].clone(), todos[2].clone()],
            ),
        );
    }
}
