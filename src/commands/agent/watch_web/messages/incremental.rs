use std::io::{Cursor, Read};
use std::path::Path;

use anyhow::Result;

use super::cache::{
    CachedTranscriptParser, IncrementalTranscriptParser, MessageCache, TranscriptChange,
    TranscriptFingerprint,
};
use super::parser;
use super::{
    ConversationResponse, Engine, TranscriptInput, open_seekable_transcript, open_transcript,
    parse_session_transcript, subagent_reports,
};

pub(super) fn parse_session_transcript_incrementally(
    session_id: &str,
    input: TranscriptInput<'_>,
    fingerprint: Option<TranscriptFingerprint>,
    cache: &MessageCache,
) -> Result<(ConversationResponse, Vec<i64>)> {
    let (Some(path), Some(fingerprint)) = (input.path, fingerprint) else {
        return parse_session_transcript(input);
    };
    if fingerprint.is_compressed() {
        return parse_session_transcript(input);
    }

    let parser_slot = cache.transcript_parser_slot(session_id);
    let mut cached = parser_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let parent_session_id = input.parent.map(|parent| parent.session_id.clone());
    let mut can_reuse = false;
    if let Some(entry) = cached.as_mut()
        && entry.engine == input.engine
        && entry.parent_session_id == parent_session_id
        && parser_matches_engine(&entry.parser, input.engine)
    {
        can_reuse = advance_cached_parser(entry, &fingerprint)?;
    }

    if can_reuse
        && let Some(entry) = cached.as_mut()
        && let Some(conversation) = render_incremental_parser(entry, path, &input)?
    {
        return Ok(conversation);
    }

    let parser = create_parser(path, &input, fingerprint.len())?;
    *cached = Some(CachedTranscriptParser {
        fingerprint,
        engine: input.engine,
        parent_session_id,
        parser,
    });
    let entry = cached
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("incremental transcript parser was not initialized"))?;
    render_incremental_parser(entry, path, &input)?
        .ok_or_else(|| anyhow::anyhow!("Claude transcript parser requires a full restart"))
}

fn parser_matches_engine(parser: &IncrementalTranscriptParser, engine: Engine) -> bool {
    matches!(
        (parser, engine),
        (IncrementalTranscriptParser::Claude(_), Engine::Claude)
            | (IncrementalTranscriptParser::Codex(_), Engine::Codex)
    )
}

fn advance_cached_parser(
    entry: &mut CachedTranscriptParser,
    fingerprint: &TranscriptFingerprint,
) -> Result<bool> {
    let (change, suffix) = fingerprint.read_delta_from(&entry.fingerprint)?;
    match change {
        TranscriptChange::Unchanged => Ok(true),
        TranscriptChange::Rewritten => Ok(false),
        TranscriptChange::Appended => {
            let appended = match &mut entry.parser {
                IncrementalTranscriptParser::Claude(parser) => matches!(
                    parser.append_bytes(&suffix, chrono::Utc::now())?,
                    parser::claude::IncrementalAppend::Applied
                ),
                IncrementalTranscriptParser::Codex(parser) => {
                    parser.append(Cursor::new(suffix.as_slice()))?;
                    true
                }
            };
            if appended {
                entry.fingerprint = fingerprint.clone();
            }
            Ok(appended)
        }
    }
}

fn create_parser(
    path: &Path,
    input: &TranscriptInput<'_>,
    captured_length: u64,
) -> Result<IncrementalTranscriptParser> {
    match input.engine {
        Engine::Claude => {
            let reader = open_seekable_transcript(path)?
                .unwrap_or_else(|| Box::new(Cursor::new(Vec::new())));
            let reader = reader.take(captured_length);
            let parser = parser::claude::IncrementalClaudeParser::new(
                reader,
                input.parent,
                input.pending_tool,
                input.watch_web_user_marker,
                chrono::Utc::now(),
                input.additions,
            )?;
            Ok(IncrementalTranscriptParser::Claude(Box::new(parser)))
        }
        Engine::Codex => {
            let mut parser = parser::codex::CodexParserState::new(input.parent);
            if let Some(reader) = open_transcript(path, false)? {
                parser.append(reader.take(captured_length))?;
            }
            Ok(IncrementalTranscriptParser::Codex(parser))
        }
    }
}

fn render_incremental_parser(
    entry: &mut CachedTranscriptParser,
    path: &Path,
    input: &TranscriptInput<'_>,
) -> Result<Option<(ConversationResponse, Vec<i64>)>> {
    match &mut entry.parser {
        IncrementalTranscriptParser::Claude(parser) => {
            let parsed = match parser.snapshot_with(
                chrono::Utc::now(),
                input.parent,
                input.pending_tool,
                input.watch_web_user_marker,
                input.additions,
            )? {
                parser::claude::IncrementalClaudeSnapshot::Parsed(parsed) => parsed,
                parser::claude::IncrementalClaudeSnapshot::RestartRequired => return Ok(None),
            };
            let (mut messages, background_tasks, resolved_queued_prompt_ids, skills) = parsed;
            subagent_reports::add_descriptions(path, &mut messages);
            Ok(Some((
                ConversationResponse {
                    messages,
                    background_tasks,
                    skills,
                    ..ConversationResponse::default()
                },
                resolved_queued_prompt_ids,
            )))
        }
        IncrementalTranscriptParser::Codex(parser) => {
            let (messages, skills) = parser.render_with_decisions_and_todos(
                input.additions.decisions,
                input.additions.todos,
            )?;
            Ok(Some((
                ConversationResponse {
                    messages,
                    skills,
                    ..ConversationResponse::default()
                },
                Vec::new(),
            )))
        }
    }
}
