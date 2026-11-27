use anyhow::Result;
use std::time::Instant;

use super::super::cache::{MessageCache, TailWindowParserState, TranscriptFingerprint};
use super::super::log_tail_window_fallback;
use super::super::parser;
use super::super::parser::cards::filter_additional_cards_after_tail_window;
use super::super::proposals::ProposalItem;
use super::super::{
    ChatItem, ConversationResponse, Engine, TranscriptInput, open_seekable_transcript,
};
use super::{TranscriptRangeContext, cached_tail_window_message_limit, render_claude_parser};
use crate::commands::agent::store::AgentTodo;

pub(in crate::commands::agent::watch_web::messages) fn parse_initial_or_full<'a>(
    session_id: &str,
    input: TranscriptInput<'a>,
    tail_window_limit: Option<usize>,
    pending_task_ids: &[String],
    fingerprint: Option<TranscriptFingerprint>,
    cache: &MessageCache,
) -> Result<(ConversationResponse, Vec<i64>, bool)> {
    parse_initial_or_full_for_range(
        session_id,
        input,
        tail_window_limit,
        pending_task_ids,
        fingerprint,
        cache,
        TranscriptRangeContext::default(),
    )
}

pub(in crate::commands::agent::watch_web::messages) fn parse_initial_or_full_for_range<'a>(
    session_id: &str,
    input: TranscriptInput<'a>,
    tail_window_limit: Option<usize>,
    pending_task_ids: &[String],
    fingerprint: Option<TranscriptFingerprint>,
    cache: &MessageCache,
    range: TranscriptRangeContext<'_>,
) -> Result<(ConversationResponse, Vec<i64>, bool)> {
    let tail_window_started = Instant::now();
    let tail_length = fingerprint
        .as_ref()
        .filter(|fingerprint| !fingerprint.is_compressed())
        .map(TranscriptFingerprint::len);
    let tail_result = match (tail_window_limit, tail_length) {
        (Some(limit), Some(file_length)) => {
            parse_session_transcript_tail_window(&input, limit, pending_task_ids, file_length)
                .map(|parsed| parsed.map(|tail| InitialTranscriptParse::Tail(Box::new(tail))))
        }
        (None, Some(file_length)) if range.older_page_limit.is_some() => {
            parse_older_page_tail_window(
                session_id,
                &input,
                pending_task_ids,
                file_length,
                range,
                cache,
            )
            .map(|parsed| parsed.map(|tail| InitialTranscriptParse::Tail(Box::new(tail))))
        }
        _ => Ok(None),
    };
    let parse_full = || {
        let range = TranscriptRangeContext {
            require_full_history: range.require_full_history || range.older_page_limit.is_some(),
            ..range
        };
        let result = if range.requested_epoch.is_none() && !range.require_full_history {
            super::parse_session_transcript_incrementally(
                session_id,
                input,
                fingerprint.clone(),
                cache,
            )
        } else {
            super::parse_session_transcript_incrementally_for_range(
                session_id,
                input,
                fingerprint.clone(),
                cache,
                range,
            )
        };
        result.map(|full| InitialTranscriptParse::Full(Box::new(full)))
    };
    let (parsed, used_tail_window) =
        parse_tail_window_or_full(tail_result, tail_window_started.elapsed(), parse_full)?;
    match parsed {
        InitialTranscriptParse::Tail(tail) => {
            let tail = *tail;
            if let Err(error) = super::cache_tail_window_parser(
                session_id,
                tail.parser,
                tail.state,
                fingerprint,
                cache,
            ) {
                tracing::warn!(session_id, %error, "failed to cache initial tail-window parser");
            }
            Ok((
                tail.conversation,
                tail.resolved_queued_prompt_ids,
                used_tail_window,
            ))
        }
        InitialTranscriptParse::Full(full) => {
            let (conversation, resolved_ids) = *full;
            Ok((conversation, resolved_ids, used_tail_window))
        }
    }
}

fn parse_older_page_tail_window(
    session_id: &str,
    input: &TranscriptInput<'_>,
    pending_task_ids: &[String],
    file_length: u64,
    range: TranscriptRangeContext<'_>,
    cache: &MessageCache,
) -> Result<Option<TailWindowParse>> {
    let (Some(epoch), Some(page_limit), Some(cached_limit)) = (
        range.requested_epoch,
        range.older_page_limit,
        cached_tail_window_message_limit(session_id, cache),
    ) else {
        return Ok(None);
    };
    if page_limit == 0 {
        return Ok(None);
    }

    let Some(mut message_limit) = cached_limit.checked_add(page_limit) else {
        return Ok(None);
    };
    let mut increment = page_limit;
    loop {
        let Some(tail) = parse_session_transcript_tail_window(
            input,
            message_limit,
            pending_task_ids,
            file_length,
        )?
        else {
            return Ok(None);
        };
        if tail.conversation.has_older_epoch_page(epoch, page_limit) {
            return Ok(Some(tail));
        }
        let Some(next_increment) = increment.checked_mul(2) else {
            return Ok(None);
        };
        let Some(next_limit) = message_limit.checked_add(next_increment) else {
            return Ok(None);
        };
        increment = next_increment;
        message_limit = next_limit;
    }
}

enum InitialTranscriptParse {
    Tail(Box<TailWindowParse>),
    Full(Box<(ConversationResponse, Vec<i64>)>),
}

fn parse_tail_window_or_full<T>(
    tail_result: Result<Option<T>>,
    tail_window_elapsed: std::time::Duration,
    full_parse: impl FnOnce() -> Result<T>,
) -> Result<(T, bool)> {
    match tail_result {
        Ok(Some(tail_result)) => Ok((tail_result, true)),
        Ok(None) => full_parse().map(|result| (result, false)),
        Err(error) if contains_io_error(&error) => {
            tracing::warn!(
                error = %error,
                reason = "tail_window_read_error",
                elapsed_ms = tail_window_elapsed.as_millis(),
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
    file_length: u64,
) -> Result<Option<TailWindowParse>> {
    let started = Instant::now();
    if input.engine != Engine::Claude
        || input
            .path
            .and_then(|path| path.extension())
            .and_then(|extension| extension.to_str())
            == Some("zst")
    {
        log_tail_window_fallback(started, "tail_window_unsupported_for_transcript");
        return Ok(None);
    }
    let reader = input
        .path
        .map(open_seekable_transcript)
        .transpose()?
        .flatten()
        .unwrap_or_else(|| Box::new(std::io::Cursor::new(Vec::new())));
    let image_directory = super::super::super::image_attachments::storage_directory().ok();
    let parser = parser::claude::IncrementalClaudeParser::new_tail_window_with_length(
        reader,
        image_directory.as_deref(),
        message_limit,
        pending_task_ids,
        file_length,
    )?;
    let Some(parser) = parser else {
        return Ok(None);
    };
    let Some(cutoff) = parser.tail_window_start() else {
        log_tail_window_fallback(started, "tail_window_start_unavailable");
        return Ok(None);
    };
    let Some(prior_resolved_ids) = resolve_queued_prompt_ids_before_window(
        input.path,
        parser.tail_window_start_offset().unwrap_or_default(),
        input.additions.queued_prompts,
        cutoff,
        input.additions.user_marker,
        &parser,
    )?
    else {
        log_tail_window_fallback(started, "queued_prompt_state_unavailable");
        return Ok(None);
    };
    let state = TailWindowParserState {
        cutoff,
        prior_resolved_ids,
        message_limit,
    };
    let (conversation, resolved_ids) = render_tail_window_parser(&parser, &state, input)?;
    // Branch filtering can leave older visible messages outside the selected suffix.
    if conversation.messages.len() < message_limit {
        return Ok(None);
    }
    Ok(Some(TailWindowParse {
        conversation,
        resolved_queued_prompt_ids: resolved_ids,
        parser,
        state,
    }))
}

pub(super) struct TailWindowParse {
    pub(super) conversation: ConversationResponse,
    pub(super) resolved_queued_prompt_ids: Vec<i64>,
    pub(super) parser: parser::claude::IncrementalClaudeParser,
    pub(super) state: TailWindowParserState,
}

pub(super) fn render_tail_window_parser(
    parser: &parser::claude::IncrementalClaudeParser,
    state: &TailWindowParserState,
    input: &TranscriptInput<'_>,
) -> Result<(ConversationResponse, Vec<i64>)> {
    let proposals = filter_proposals_after_tail_window(input.additions.proposals, state.cutoff);
    let todos = filter_todos_after_tail_window(input.additions.todos, state.cutoff);
    let peer_out_messages = input
        .additions
        .peer_out_messages
        .iter()
        .filter(|message| message.sent_at >= state.cutoff)
        .cloned()
        .collect::<Vec<_>>();
    let additional_cards =
        filter_additional_cards_after_tail_window(input.additions.additional_cards, state.cutoff);
    let prior_resolved = state
        .prior_resolved_ids
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let queued_prompts = input
        .additions
        .queued_prompts
        .iter()
        .filter(|prompt| !prior_resolved.contains(&prompt.id))
        .cloned()
        .collect::<Vec<_>>();
    let tail_input = TranscriptInput {
        path: input.path,
        engine: input.engine,
        additions: parser::claude::ParserAdditions {
            proposals: &proposals,
            todos: &todos,
            peer_out_messages: &peer_out_messages,
            additional_cards: &additional_cards,
            queued_prompts: &queued_prompts,
            ..input.additions
        },
    };
    let mut result = render_claude_parser(parser, &tail_input)?;
    result.1.extend(state.prior_resolved_ids.iter().copied());
    result.1.sort_unstable();
    result.1.dedup();
    result.0.tail_window_start = Some(state.cutoff);
    Ok(result)
}

fn filter_proposals_after_tail_window(
    proposals: &[ProposalItem],
    cutoff: chrono::DateTime<chrono::Utc>,
) -> Vec<ProposalItem> {
    proposals
        .iter()
        .filter(|proposal| {
            matches!(
                &proposal.item,
                ChatItem::Proposal {
                    resolved: false,
                    ..
                }
            ) || proposal.position_timestamp() >= cutoff
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
        .filter(|todo| {
            todo.completed_at
                .is_none_or(|completed_at| completed_at >= cutoff)
        })
        .cloned()
        .collect()
}

fn resolve_queued_prompt_ids_before_window(
    path: Option<&std::path::Path>,
    window_start: u64,
    queued_prompts: &[crate::commands::agent::store::WatchWebQueuedPrompt],
    cutoff: chrono::DateTime<chrono::Utc>,
    user_marker: Option<&str>,
    parser: &parser::claude::IncrementalClaudeParser,
) -> Result<Option<Vec<i64>>> {
    let Some(path) = path else {
        return Ok(Some(Vec::new()));
    };
    let unresolved_prompts = queued_prompts
        .iter()
        .filter(|prompt| !prompt.resolved && prompt.sent_at < cutoff)
        .cloned()
        .collect::<Vec<_>>();
    if unresolved_prompts.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let Some(mut reader) = open_seekable_transcript(path)? else {
        return Ok(Some(Vec::new()));
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
    Ok(Some(prior_resolved_ids))
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::{TailWindowParserState, parse_tail_window_or_full, render_tail_window_parser};
    use crate::commands::agent::proposal::{ProposalAnswer, ProposalOption};
    use crate::commands::agent::store::{AgentTodo, TodoResolution};
    use crate::commands::agent::types::Engine;
    use crate::commands::agent::watch_web::messages::parser::claude::{
        IncrementalClaudeParser, ParserAdditions,
    };
    use crate::commands::agent::watch_web::messages::proposals::ProposalItem;
    use crate::commands::agent::watch_web::messages::{
        ChatItem, ConversationResponse, TranscriptInput,
    };
    use indoc::indoc;
    use std::io::Cursor;

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
            std::time::Duration::ZERO,
            || Ok(full_result.clone()),
        )
        .unwrap();

        assert_eq!(actual, (full_result, false));
    }

    #[test]
    fn tail_window_renders_pending_cards_and_recently_resolved_cards() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-before","parentUuid":null,"timestamp":"2026-06-01T11:59:00Z","type":"assistant","message":{"content":[{"type":"text","text":"Before"}]}}
            {"uuid":"fixture-after","parentUuid":"fixture-before","timestamp":"2026-06-01T12:01:00Z","type":"assistant","message":{"content":[{"type":"text","text":"After"}]}}
        "#};
        let parser = IncrementalClaudeParser::new(Cursor::new(transcript), None)
            .expect("Claude transcript should parse");
        let cutoff = timestamp("2026-06-01T12:00:00Z");
        let proposals = [
            ProposalItem {
                seq: 1,
                created_at: timestamp("2026-06-01T11:59:56Z"),
                resolved_at: Some(timestamp("2026-06-01T11:59:59Z")),
                item: ChatItem::Proposal {
                    seq: 1,
                    title: "An old resolved proposal".to_string(),
                    summary: None,
                    options: Vec::new(),
                    answer: None,
                    resolved: true,
                    resolution: None,
                    comment: None,
                },
            },
            ProposalItem {
                seq: 2,
                created_at: timestamp("2026-06-01T11:59:56Z"),
                resolved_at: Some(timestamp("2026-06-01T12:00:01Z")),
                item: ChatItem::Proposal {
                    seq: 2,
                    title: "A recently resolved proposal".to_string(),
                    summary: None,
                    options: Vec::new(),
                    answer: Some(ProposalAnswer {
                        choice: Some(0),
                        comment: None,
                    }),
                    resolved: true,
                    resolution: None,
                    comment: None,
                },
            },
            ProposalItem {
                seq: 3,
                created_at: timestamp("2026-06-01T11:59:57Z"),
                resolved_at: None,
                item: ChatItem::Proposal {
                    seq: 3,
                    title: "A pending proposal".to_string(),
                    summary: None,
                    options: vec![ProposalOption {
                        label: "Option A".to_string(),
                        description: None,
                        pros: Vec::new(),
                        cons: Vec::new(),
                        recommended: false,
                    }],
                    answer: None,
                    resolved: false,
                    resolution: None,
                    comment: None,
                },
            },
        ];
        let todos = [
            AgentTodo {
                session_id: "fixture-session".to_string(),
                seq: 1,
                title: "An old incomplete todo".to_string(),
                detail: None,
                folded: false,
                added_folded: false,
                created_at: timestamp("2026-06-01T11:59:58Z"),
                completed_at: None,
                resolution: None,
                comment: None,
            },
            AgentTodo {
                session_id: "fixture-session".to_string(),
                seq: 2,
                title: "An old completed todo".to_string(),
                detail: None,
                folded: false,
                added_folded: false,
                created_at: timestamp("2026-06-01T11:59:57Z"),
                completed_at: Some(timestamp("2026-06-01T11:59:59Z")),
                resolution: Some(TodoResolution::Completed),
                comment: None,
            },
            AgentTodo {
                session_id: "fixture-session".to_string(),
                seq: 3,
                title: "A recently completed todo".to_string(),
                detail: None,
                folded: false,
                added_folded: false,
                created_at: timestamp("2026-06-01T11:59:56Z"),
                completed_at: Some(timestamp("2026-06-01T12:00:02Z")),
                resolution: Some(TodoResolution::Completed),
                comment: None,
            },
        ];
        let additions = ParserAdditions {
            session_id: "fixture-session",
            parent: None,
            pending_tool: None,
            user_marker: None,
            queued_prompts: &[],
            peer_out_messages: &[],
            proposals: &proposals,
            todos: &todos,
            additional_cards: &[],
        };
        let input = TranscriptInput {
            path: None,
            engine: Engine::Claude,
            additions,
        };
        let state = TailWindowParserState {
            cutoff,
            prior_resolved_ids: Vec::new(),
            message_limit: 2,
        };
        let (conversation, resolved_ids) =
            render_tail_window_parser(&parser, &state, &input).expect("tail window should render");

        assert_eq!(
            (conversation, resolved_ids),
            (
                ConversationResponse {
                    messages: vec![
                        ChatItem::Assistant {
                            text: "Before".to_string(),
                        },
                        proposals[1].item.clone(),
                        ChatItem::Todo {
                            seq: 3,
                            title: "A recently completed todo".to_string(),
                            detail: None,
                            folded: false,
                            added_folded: false,
                            completed: true,
                            resolution: Some(TodoResolution::Completed),
                            comment: None,
                        },
                        ChatItem::Assistant {
                            text: "After".to_string(),
                        },
                        proposals[2].item.clone(),
                        ChatItem::Todo {
                            seq: 1,
                            title: "An old incomplete todo".to_string(),
                            detail: None,
                            folded: false,
                            added_folded: false,
                            completed: false,
                            resolution: None,
                            comment: None,
                        },
                    ],
                    tail_window_start: Some(cutoff),
                    ..ConversationResponse::default()
                },
                Vec::new(),
            ),
        );
    }
}
