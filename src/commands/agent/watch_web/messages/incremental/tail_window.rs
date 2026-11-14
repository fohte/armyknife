use std::io::{Read, Seek, SeekFrom};

use anyhow::Result;
use memchr::memmem;

use super::super::cache::IncrementalTranscriptParser;
use super::super::parser;
use super::super::{ConversationResponse, Engine, TranscriptInput, open_seekable_transcript};
use super::render_parser;

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
    )?
    else {
        return Ok(None);
    };
    let decisions = input
        .additions
        .decisions
        .iter()
        .filter(|decision| {
            chrono::DateTime::parse_from_rfc3339(&decision.created_at)
                .is_ok_and(|created_at| created_at.with_timezone(&chrono::Utc) >= cutoff)
        })
        .cloned()
        .collect::<Vec<_>>();
    let proposals = input
        .additions
        .proposals
        .iter()
        .filter(|proposal| proposal.created_at >= cutoff)
        .cloned()
        .collect::<Vec<_>>();
    let todos = input
        .additions
        .todos
        .iter()
        .filter(|todo| todo.created_at >= cutoff)
        .cloned()
        .collect::<Vec<_>>();
    let peer_out_messages = input
        .additions
        .peer_out_messages
        .iter()
        .filter(|message| message.sent_at >= cutoff)
        .cloned()
        .collect::<Vec<_>>();
    let tail_input = TranscriptInput {
        path: input.path,
        engine: input.engine,
        additions: parser::claude::ParserAdditions {
            decisions: &decisions,
            proposals: &proposals,
            todos: &todos,
            peer_out_messages: &peer_out_messages,
            queued_prompts: &queued_prompts,
            ..input.additions
        },
    };
    let (mut conversation, mut resolved_ids) = render_parser(
        &IncrementalTranscriptParser::Claude(Box::new(parser)),
        &tail_input,
    )?;
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
) -> Result<
    Option<(
        Vec<crate::commands::agent::store::WatchWebQueuedPrompt>,
        Vec<i64>,
    )>,
> {
    const MAX_PREFIX_SEARCH_BYTES: u64 = 256 * 1024 * 1024;
    const MAX_MARKER_SEARCHES: usize = 4096;

    let Some(path) = path else {
        return Ok(Some((queued_prompts.to_vec(), Vec::new())));
    };
    if window_start > MAX_PREFIX_SEARCH_BYTES {
        return Ok(None);
    }
    let unresolved_prompts = queued_prompts
        .iter()
        .filter(|prompt| !prompt.resolved && prompt.sent_at < cutoff)
        .collect::<Vec<_>>();
    if unresolved_prompts.len() > MAX_MARKER_SEARCHES {
        return Ok(None);
    }
    let Some(mut reader) = open_seekable_transcript(path)? else {
        return Ok(Some((queued_prompts.to_vec(), Vec::new())));
    };
    let mut prior_resolved_ids = Vec::new();
    for prompt in unresolved_prompts {
        let Some(message_id) = prompt.message_id.as_deref() else {
            continue;
        };
        let marker = format!("<!-- armyknife-queued-prompt-id:{message_id} -->");
        if prefix_contains(&mut reader, window_start, marker.as_bytes())? {
            prior_resolved_ids.push(prompt.id);
        }
    }
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

fn prefix_contains(
    reader: &mut (impl Read + Seek),
    prefix_length: u64,
    needle: &[u8],
) -> Result<bool> {
    const CHUNK_BYTES: u64 = 64 * 1024;

    if needle.is_empty() {
        return Ok(true);
    }
    let mut offset = 0;
    let mut overlap = Vec::new();
    while offset < prefix_length {
        let chunk_length = CHUNK_BYTES.min(prefix_length - offset);
        let mut chunk = vec![0; usize::try_from(chunk_length)?];
        reader.seek(SeekFrom::Start(offset))?;
        reader.read_exact(&mut chunk)?;
        overlap.extend_from_slice(&chunk);
        if memmem::find(&overlap, needle).is_some() {
            return Ok(true);
        }
        let keep = needle.len().saturating_sub(1).min(overlap.len());
        overlap.drain(..overlap.len() - keep);
        offset += chunk_length;
    }
    Ok(false)
}
