use std::io::{Cursor, Read};

use anyhow::Result;

use super::cache::{
    CachedTranscriptParser, IncrementalTranscriptParser, MessageCache, TranscriptChange,
    TranscriptFingerprint,
};
use super::parser;
use super::{
    ConversationResponse, Engine, TranscriptInput, open_seekable_transcript, open_transcript,
    subagent_reports,
};

pub(super) fn parse_session_transcript(
    input: TranscriptInput<'_>,
) -> Result<(ConversationResponse, Vec<i64>)> {
    let parser = create_parser(&input, None)?;
    render_parser(&parser, &input)
}

pub(super) fn parse_session_transcript_incrementally(
    session_id: &str,
    input: TranscriptInput<'_>,
    fingerprint: Option<TranscriptFingerprint>,
    cache: &MessageCache,
) -> Result<(ConversationResponse, Vec<i64>)> {
    let Some(fingerprint) = fingerprint else {
        clear_parser(cache, session_id);
        return parse_session_transcript(input);
    };
    let parser_slot = cache.transcript_parser_slot(session_id);
    let mut cached = parser_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let can_reuse = if let Some(entry) = cached.as_mut()
        && entry.engine == input.engine
        && parser_matches_engine(&entry.parser, input.engine)
    {
        match advance_cached_parser(entry, &fingerprint) {
            Ok(can_reuse) => can_reuse,
            Err(error) => {
                *cached = None;
                return Err(error);
            }
        }
    } else {
        false
    };

    if can_reuse {
        let entry = cached
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("incremental transcript parser was not initialized"))?;
        return match render_parser(&entry.parser, &input) {
            Ok(conversation) => Ok(conversation),
            Err(error) => {
                *cached = None;
                Err(error)
            }
        };
    }

    *cached = None;
    let captured_length = (!fingerprint.is_compressed()).then(|| fingerprint.len());
    let parser = create_parser(&input, captured_length)?;
    let tail = if fingerprint.is_compressed() {
        Vec::new()
    } else {
        let Some(tail) = fingerprint.tail_bytes()? else {
            return parse_session_transcript(input);
        };
        tail
    };
    *cached = Some(CachedTranscriptParser {
        fingerprint,
        tail,
        engine: input.engine,
        #[cfg(test)]
        last_change: TranscriptChange::Rewritten,
        parser,
    });
    let entry = cached
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("incremental transcript parser was not initialized"))?;
    match render_parser(&entry.parser, &input) {
        Ok(conversation) => Ok(conversation),
        Err(error) => {
            *cached = None;
            Err(error)
        }
    }
}

fn clear_parser(cache: &MessageCache, session_id: &str) {
    let slot = cache.transcript_parser_slot(session_id);
    *slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
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
    let (change, suffix) = fingerprint.read_delta_from(&entry.fingerprint, &entry.tail)?;
    match change {
        TranscriptChange::Unchanged => {
            #[cfg(test)]
            {
                entry.last_change = change;
            }
            Ok(true)
        }
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
                let Some(tail) = fingerprint.tail_bytes()? else {
                    return Ok(false);
                };
                entry.tail = tail;
                entry.fingerprint = fingerprint.clone();
                #[cfg(test)]
                {
                    entry.last_change = change;
                }
            }
            Ok(appended)
        }
    }
}

fn create_parser(
    input: &TranscriptInput<'_>,
    captured_length: Option<u64>,
) -> Result<IncrementalTranscriptParser> {
    match input.engine {
        Engine::Claude => {
            let reader = input
                .path
                .map(open_seekable_transcript)
                .transpose()?
                .flatten()
                .unwrap_or_else(|| Box::new(Cursor::new(Vec::new())));
            let image_directory = super::super::image_attachments::storage_directory().ok();
            let parser = match captured_length {
                Some(length) => parser::claude::IncrementalClaudeParser::new(
                    reader.take(length),
                    image_directory.as_deref(),
                )?,
                None => parser::claude::IncrementalClaudeParser::new(
                    reader,
                    image_directory.as_deref(),
                )?,
            };
            Ok(IncrementalTranscriptParser::Claude(Box::new(parser)))
        }
        Engine::Codex => {
            let reader = input
                .path
                .map(|path| open_transcript(path, captured_length.is_none()))
                .transpose()?
                .flatten();
            let mut parser = parser::codex::CodexParserState::new();
            if let Some(reader) = reader {
                match captured_length {
                    Some(length) => parser.append(reader.take(length))?,
                    None => parser.append(reader)?,
                }
            }
            Ok(IncrementalTranscriptParser::Codex(parser))
        }
    }
}

fn render_parser(
    parser: &IncrementalTranscriptParser,
    input: &TranscriptInput<'_>,
) -> Result<(ConversationResponse, Vec<i64>)> {
    match parser {
        IncrementalTranscriptParser::Claude(parser) => {
            let (mut messages, background_tasks, resolved_ids, skills) =
                parser.render_with(chrono::Utc::now(), input.additions)?;
            if let Some(path) = input.path {
                subagent_reports::add_descriptions(path, &mut messages);
            }
            Ok((
                ConversationResponse {
                    messages,
                    background_tasks,
                    skills,
                    ..ConversationResponse::default()
                },
                resolved_ids,
            ))
        }
        IncrementalTranscriptParser::Codex(parser) => {
            let (messages, skills) = parser.render_with_decisions_todos_and_peer_out_messages(
                input.additions.parent,
                input.additions.decisions,
                input.additions.todos,
                input.additions.peer_out_messages,
            )?;
            Ok((
                ConversationResponse {
                    messages,
                    skills,
                    ..ConversationResponse::default()
                },
                Vec::new(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_session_transcript_incrementally;
    use crate::commands::agent::types::{Engine, Session};
    use crate::commands::agent::watch_web::messages::cache::{
        MessageCache, RunningBackgroundTaskCache,
    };
    use crate::commands::agent::watch_web::messages::parser::claude::ParserAdditions;
    use crate::commands::agent::watch_web::messages::{ConversationResponse, TranscriptInput};

    #[test]
    fn missing_transcript_after_fingerprinting_renders_an_empty_conversation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.jsonl");
        std::fs::write(
            &path,
            br#"{"type":"assistant","message":{"content":[{"type":"text","text":"Fixture response."}]}}"#,
        )
        .unwrap();
        let session: Session = serde_json::from_value(serde_json::json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "status": "ended",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "engine": "claude",
            "transcript_path": path,
        }))
        .unwrap();
        let fingerprint = RunningBackgroundTaskCache::fingerprint(&session)
            .unwrap()
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        let result = parse_session_transcript_incrementally(
            "fixture-session",
            TranscriptInput {
                path: Some(&path),
                engine: Engine::Claude,
                additions: ParserAdditions {
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
            },
            Some(fingerprint),
            &MessageCache::default(),
        )
        .unwrap();

        assert_eq!(result, (ConversationResponse::default(), Vec::new()));
    }
}
