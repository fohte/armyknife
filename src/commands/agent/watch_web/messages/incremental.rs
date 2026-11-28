use std::io::{Cursor, Read};
use std::time::Instant;

use anyhow::Result;

use super::cache::{
    CachedTranscriptParser, IncrementalTranscriptParser, MessageCache, TailWindowParserState,
    TranscriptChange, TranscriptFingerprint,
};
use super::parser;
use super::{
    ConversationResponse, Engine, TranscriptInput, open_seekable_transcript, open_transcript,
    subagent_reports,
};

mod tail_window;

pub(super) use tail_window::filter_after_tail_window;
pub(super) use tail_window::parse_initial_or_full;

pub(super) use tail_window::parse_initial_or_full_for_range;

#[derive(Clone, Copy, Default)]
pub(super) struct TranscriptRangeContext<'a> {
    pub(super) requested_epoch: Option<&'a str>,
    pub(super) older_page_limit: Option<usize>,
    pub(super) require_full_history: bool,
}

pub(super) fn cached_tail_window_message_limit(
    session_id: &str,
    cache: &MessageCache,
) -> Option<usize> {
    let parser_slot = cache.transcript_parser_slot(session_id);
    let cached = parser_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cached
        .as_ref()
        .filter(|entry| entry.engine == Engine::Claude)
        .and_then(|entry| entry.tail_window.as_ref())
        .map(|state| state.message_limit)
}

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
    pending_task_ids: &[String],
    cache: &MessageCache,
) -> Result<(ConversationResponse, Vec<i64>)> {
    parse_session_transcript_incrementally_for_range(
        session_id,
        input,
        fingerprint,
        pending_task_ids,
        cache,
        TranscriptRangeContext::default(),
    )
}

pub(super) fn parse_session_transcript_incrementally_for_range(
    session_id: &str,
    input: TranscriptInput<'_>,
    fingerprint: Option<TranscriptFingerprint>,
    pending_task_ids: &[String],
    cache: &MessageCache,
    range: TranscriptRangeContext<'_>,
) -> Result<(ConversationResponse, Vec<i64>)> {
    let Some(fingerprint) = fingerprint else {
        clear_parser(cache, session_id);
        let parse_started = Instant::now();
        let parser = create_parser(&input, None)?;
        tracing::info!(
            session_id,
            reason = "transcript_fingerprint_unavailable",
            elapsed_ms = parse_started.elapsed().as_millis(),
            "incremental transcript parser restarted with a full parse"
        );
        return render_parser(&parser, &input);
    };
    let parser_slot = cache.transcript_parser_slot(session_id);
    let mut cached = parser_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut restart_reason = Some("cache_miss");
    let can_reuse = if let Some(entry) = cached.as_mut()
        && entry.engine == input.engine
        && parser_matches_engine(&entry.parser, input.engine)
    {
        if entry.tail_window.is_some() && range.require_full_history {
            restart_reason = Some("request_requires_full_history");
            false
        } else {
            match advance_cached_parser(entry, &fingerprint) {
                Ok(ParserAdvance::Reuse) => true,
                Ok(ParserAdvance::Restart(reason)) => {
                    restart_reason = Some(reason);
                    false
                }
                Err(error) => {
                    *cached = None;
                    return Err(error);
                }
            }
        }
    } else {
        if cached
            .as_ref()
            .is_some_and(|entry| entry.engine != input.engine)
        {
            restart_reason = Some("parser_engine_changed");
        } else if cached
            .as_ref()
            .is_some_and(|entry| !parser_matches_engine(&entry.parser, input.engine))
        {
            restart_reason = Some("parser_engine_mismatch");
        }
        false
    };

    if can_reuse {
        let rendered = {
            let entry = cached.as_ref().ok_or_else(|| {
                anyhow::anyhow!("incremental transcript parser was not initialized")
            })?;
            match (&entry.parser, &entry.tail_window) {
                (IncrementalTranscriptParser::Claude(parser), Some(state)) => {
                    tail_window::render_tail_window_parser(parser, state, &input)
                }
                _ => render_parser(&entry.parser, &input),
            }
        };
        match rendered {
            Ok((mut conversation, resolved_ids)) => {
                let tail_window_cached = cached
                    .as_ref()
                    .is_some_and(|entry| entry.tail_window.is_some());
                if tail_window_cached {
                    let cutoff = cached
                        .as_ref()
                        .and_then(|entry| entry.tail_window.as_ref())
                        .map(|state| state.cutoff);
                    conversation.tail_window_start = cutoff;
                }
                let missing_anchor = tail_window_cached
                    && range
                        .requested_epoch
                        .is_some_and(|epoch| !conversation.epoch_anchor_is_present(epoch));
                if !missing_anchor {
                    return Ok((conversation, resolved_ids));
                }
                restart_reason = Some("epoch_anchor_outside_tail_window");
                *cached = None;
            }
            Err(error) => {
                *cached = None;
                return Err(error);
            }
        }
    }

    if should_rebuild_tail_window(
        cached
            .as_ref()
            .is_some_and(|entry| entry.tail_window.is_some()),
        range,
        restart_reason,
        fingerprint.is_compressed(),
    ) {
        let message_limit = cached
            .as_ref()
            .and_then(|entry| entry.tail_window.as_ref())
            .map_or(super::range::INITIAL_EPOCH_MESSAGE_COUNT, |state| {
                state.message_limit
            });
        if fingerprint.tail_bytes()?.is_some() {
            match tail_window::rebuild_tail_window_parser(
                &input,
                pending_task_ids,
                &fingerprint,
                range,
                message_limit,
            )? {
                tail_window::TailWindowRebuild::Rebuilt(tail) => {
                    let tail_window::TailWindowParse {
                        conversation,
                        resolved_queued_prompt_ids,
                        parser,
                        state,
                    } = *tail;
                    if let Some(tail) = fingerprint.tail_bytes()? {
                        *cached = Some(CachedTranscriptParser {
                            fingerprint: fingerprint.clone(),
                            tail,
                            engine: Engine::Claude,
                            tail_window: Some(state),
                            #[cfg(test)]
                            last_change: TranscriptChange::Rewritten,
                            parser: IncrementalTranscriptParser::Claude(Box::new(parser)),
                        });
                        return Ok((conversation, resolved_queued_prompt_ids));
                    } else {
                        restart_reason = Some("transcript_changed_during_tail_rebuild");
                    }
                }
                tail_window::TailWindowRebuild::AnchorMissing => {
                    restart_reason = Some("epoch_anchor_outside_tail_window");
                }
                tail_window::TailWindowRebuild::Unavailable => {}
            }
        } else {
            restart_reason = Some("transcript_tail_unavailable");
        }
    }

    *cached = None;
    let captured_length = (!fingerprint.is_compressed()).then(|| fingerprint.len());
    let parse_started = Instant::now();
    let parser = create_parser(&input, captured_length)?;
    tracing::info!(
        session_id,
        reason = restart_reason.unwrap_or("incremental_parser_unavailable"),
        elapsed_ms = parse_started.elapsed().as_millis(),
        "incremental transcript parser restarted with a full parse"
    );
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
        tail_window: None,
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

pub(super) fn cache_tail_window_parser(
    session_id: &str,
    parser: parser::claude::IncrementalClaudeParser,
    state: TailWindowParserState,
    fingerprint: Option<TranscriptFingerprint>,
    cache: &MessageCache,
) -> Result<()> {
    let Some(fingerprint) = fingerprint else {
        return Ok(());
    };
    let Some(tail) = fingerprint.tail_bytes()? else {
        return Ok(());
    };
    let parser_slot = cache.transcript_parser_slot(session_id);
    *parser_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(CachedTranscriptParser {
        fingerprint,
        tail,
        engine: Engine::Claude,
        tail_window: Some(state),
        #[cfg(test)]
        last_change: TranscriptChange::Unchanged,
        parser: IncrementalTranscriptParser::Claude(Box::new(parser)),
    });
    Ok(())
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

fn should_rebuild_tail_window(
    cached_tail_window: bool,
    range: TranscriptRangeContext<'_>,
    restart_reason: Option<&str>,
    compressed: bool,
) -> bool {
    !range.require_full_history
        && range
            .requested_epoch
            .is_none_or(super::range::epoch_token_is_decodable)
        && (cached_tail_window || range.requested_epoch.is_some())
        && restart_reason != Some("epoch_anchor_outside_tail_window")
        && !compressed
}

fn advance_cached_parser(
    entry: &mut CachedTranscriptParser,
    fingerprint: &TranscriptFingerprint,
) -> Result<ParserAdvance> {
    let (change, suffix) = fingerprint.read_delta_from(&entry.fingerprint, &entry.tail)?;
    match change {
        TranscriptChange::Unchanged => {
            #[cfg(test)]
            {
                entry.last_change = change;
            }
            Ok(ParserAdvance::Reuse)
        }
        TranscriptChange::Rewritten => Ok(ParserAdvance::Restart("transcript_rewritten")),
        TranscriptChange::Appended => {
            let restart_reason = match &mut entry.parser {
                IncrementalTranscriptParser::Claude(parser) => {
                    match parser.append_bytes(&suffix, chrono::Utc::now())? {
                        parser::claude::IncrementalAppend::Applied => None,
                        parser::claude::IncrementalAppend::RestartRequired => {
                            Some("append_requires_branch_reparse")
                        }
                    }
                }
                IncrementalTranscriptParser::Codex(parser) => {
                    parser.append(Cursor::new(suffix.as_slice()))?;
                    None
                }
            };
            if let Some(reason) = restart_reason {
                return Ok(ParserAdvance::Restart(reason));
            }
            let Some(tail) = fingerprint.tail_bytes()? else {
                return Ok(ParserAdvance::Restart("transcript_tail_unavailable"));
            };
            entry.tail = tail;
            entry.fingerprint = fingerprint.clone();
            #[cfg(test)]
            {
                entry.last_change = change;
            }
            Ok(ParserAdvance::Reuse)
        }
    }
}

enum ParserAdvance {
    Reuse,
    Restart(&'static str),
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
        IncrementalTranscriptParser::Claude(parser) => render_claude_parser(parser, input),
        IncrementalTranscriptParser::Codex(parser) => {
            let placement_cards = parser::cards::cards_from_sources(
                input.additions.proposals,
                input.additions.todos,
                input.additions.peer_out_messages,
                input.additions.additional_cards,
            );
            let (mut messages, skills) = parser.render_with_placement_cards(
                input.additions.parent,
                &placement_cards,
                input.additions.pending_tool,
            )?;
            let resolved_ids = super::queued_prompt::resolve_transcript_messages(
                &mut messages,
                input.additions.queued_prompts,
            );
            super::queued_prompt::append_unresolved_prompts(
                &mut messages,
                input.additions.queued_prompts,
                &resolved_ids,
            );
            Ok((
                ConversationResponse {
                    messages,
                    skills,
                    ..ConversationResponse::default()
                },
                resolved_ids,
            ))
        }
    }
}

pub(super) fn render_claude_parser(
    parser: &parser::claude::IncrementalClaudeParser,
    input: &TranscriptInput<'_>,
) -> Result<(ConversationResponse, Vec<i64>)> {
    let (mut messages, background_tasks, mut resolved_ids, skills) =
        parser.render_with(chrono::Utc::now(), input.additions)?;
    resolved_ids.extend(super::queued_prompt::resolve_transcript_messages(
        &mut messages,
        input.additions.queued_prompts,
    ));
    resolved_ids.sort_unstable();
    resolved_ids.dedup();
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

#[cfg(test)]
mod tests {
    use super::{parse_session_transcript, parse_session_transcript_incrementally};
    use crate::commands::agent::store::{WatchWebQueuedAttachment, WatchWebQueuedPrompt};
    use crate::commands::agent::types::Engine;
    use crate::commands::agent::watch_web::messages::cache::{
        MessageCache, transcript_fingerprint,
    };
    use crate::commands::agent::watch_web::messages::parser::claude::ParserAdditions;
    use crate::commands::agent::watch_web::messages::range::MessageRange;
    use crate::commands::agent::watch_web::messages::{
        ChatItem, ConversationResponse, TranscriptInput, UserContentPart,
    };
    use chrono::{DateTime, Utc};
    use indoc::indoc;

    #[rstest::rstest]
    #[case::claude(Engine::Claude)]
    #[case::codex(Engine::Codex)]
    fn resolves_duplicate_watch_web_messages_by_id_for_each_engine(#[case] engine: Engine) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.jsonl");
        let session_marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let prompt_contents = [
            ("fixture-message-cyan", "First attached file."),
            ("fixture-message-amber", "Second attached file."),
        ];
        let transcript = match engine {
            Engine::Claude => {
                let mut records = vec![format!(
                    r#"{{"type":"user","origin":{{"kind":"peer","from":"unknown"}},"message":{{"content":"{session_marker}\nThis message was typed by the human user in armyknife watch-web.\nOld message."}}}}"#
                )];
                records.extend(prompt_contents.iter().map(|(message_id, _)| {
                    format!(
                        r#"{{"type":"user","origin":{{"kind":"human"}},"message":{{"content":"{session_marker}\nThis message was typed by the human user in armyknife watch-web.\nEngine-formatted request.\n<!-- armyknife-queued-prompt-id:{message_id} -->"}}}}"#
                    )
                }));
                records.join("\n")
            }
            Engine::Codex => {
                let mut records = vec![
                    r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":"Old message."}}}"#.to_owned(),
                ];
                records.extend(prompt_contents.iter().map(|(message_id, _)| {
                    format!(
                        r#"{{"type":"event_msg","payload":{{"type":"item_completed","item":{{"type":"UserMessage","content":"Engine-formatted request.\n<!-- armyknife-queued-prompt-id:{message_id} -->"}}}}}}"#
                    )
                }));
                records.join("\n")
            }
        };
        std::fs::write(&path, transcript).unwrap();
        let sent_at = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let queued_prompts = prompt_contents
            .iter()
            .enumerate()
            .map(|(index, (message_id, filename))| WatchWebQueuedPrompt {
                id: index as i64 + 1,
                message_id: Some((*message_id).to_owned()),
                content: "Repeat this request.".to_owned(),
                action_event_kind: None,
                attachments: vec![WatchWebQueuedAttachment::File {
                    name: (*filename).to_owned(),
                }],
                resolved: false,
                sent_at,
            })
            .collect::<Vec<_>>();
        let parse = |prompts: &[WatchWebQueuedPrompt]| {
            parse_session_transcript(TranscriptInput {
                path: Some(&path),
                engine,
                additions: ParserAdditions {
                    session_id: "fixture-session",
                    parent: None,
                    pending_tool: None,
                    user_marker: (engine == Engine::Claude).then_some(session_marker),
                    queued_prompts: prompts,
                    peer_out_messages: &[],
                    proposals: &[],
                    todos: &[],
                    additional_cards: &[],
                },
            })
            .unwrap()
        };
        let first_parse = parse(&queued_prompts);
        let mut resolved_prompts = queued_prompts.clone();
        for prompt in &mut resolved_prompts {
            prompt.resolved = true;
        }
        let subsequent_parse = parse(&resolved_prompts);
        let expected_messages = vec![
            ChatItem::User {
                rewind_id: None,
                text: "Old message.".to_owned(),
                from: None,
                timestamp: None,
                queued_prompt_id: None,
            },
            ChatItem::UserWithContent {
                rewind_id: None,
                content: vec![
                    UserContentPart::Text {
                        text: "Repeat this request.".to_owned(),
                    },
                    UserContentPart::File {
                        name: "First attached file.".to_owned(),
                    },
                ],
                from: None,
                timestamp: None,
                queued_prompt_id: Some("fixture-message-cyan".to_owned()),
            },
            ChatItem::UserWithContent {
                rewind_id: None,
                content: vec![
                    UserContentPart::Text {
                        text: "Repeat this request.".to_owned(),
                    },
                    UserContentPart::File {
                        name: "Second attached file.".to_owned(),
                    },
                ],
                from: None,
                timestamp: None,
                queued_prompt_id: Some("fixture-message-amber".to_owned()),
            },
        ];

        assert_eq!(
            (
                (first_parse.0.messages, first_parse.1),
                (subsequent_parse.0.messages, subsequent_parse.1),
            ),
            (
                (expected_messages.clone(), vec![1, 2]),
                (expected_messages, Vec::new()),
            ),
        );
    }

    #[test]
    fn missing_transcript_after_fingerprinting_renders_an_empty_conversation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.jsonl");
        std::fs::write(
            &path,
            br#"{"type":"assistant","message":{"content":[{"type":"text","text":"Fixture response."}]}}"#,
        )
        .unwrap();
        let fingerprint = transcript_fingerprint(Engine::Claude, Some(&path))
            .unwrap()
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        let result = parse_session_transcript_incrementally(
            "fixture-session",
            TranscriptInput {
                path: Some(&path),
                engine: Engine::Claude,
                additions: ParserAdditions {
                    session_id: "fixture-session",
                    parent: None,
                    pending_tool: None,
                    user_marker: None,
                    queued_prompts: &[],
                    peer_out_messages: &[],
                    proposals: &[],
                    todos: &[],
                    additional_cards: &[],
                },
            },
            Some(fingerprint),
            &[],
            &MessageCache::default(),
        )
        .unwrap();

        assert_eq!(result, (ConversationResponse::default(), Vec::new()));
    }

    #[rstest::rstest]
    fn cached_tail_window_rebuilds_after_a_branch_switch(epoch_transcript: tempfile::TempDir) {
        let path = epoch_transcript.path().join("fixture.jsonl");
        let cache = MessageCache::default();
        let initial_fingerprint = transcript_fingerprint(Engine::Claude, Some(&path))
            .unwrap()
            .unwrap();
        super::tail_window::parse_initial_or_full_for_range(
            "fixture-session",
            epoch_input(&path),
            Some(100),
            &[],
            Some(initial_fingerprint),
            &cache,
            super::TranscriptRangeContext::default(),
        )
        .unwrap();

        let mut transcript = std::fs::read_to_string(&path).unwrap();
        transcript.push('\n');
        transcript.push_str(
            indoc! {r#"
                {"type":"user","uuid":"fixture-rewind-request","parentUuid":"fixture-assistant-50","timestamp":"2026-01-01T01:00:00Z","message":{"content":"Continue from an earlier request."}}
                {"type":"assistant","uuid":"fixture-rewind-response","parentUuid":"fixture-rewind-request","timestamp":"2026-01-01T01:00:01Z","message":{"content":[{"type":"text","text":"The earlier branch continues."}]}}
            "#}
            .trim(),
        );
        std::fs::write(&path, transcript).unwrap();

        let fingerprint = transcript_fingerprint(Engine::Claude, Some(&path))
            .unwrap()
            .unwrap();
        let (mut actual, resolved_ids, _) = super::tail_window::parse_initial_or_full_for_range(
            "fixture-session",
            epoch_input(&path),
            None,
            &[],
            Some(fingerprint),
            &cache,
            super::TranscriptRangeContext::default(),
        )
        .unwrap();
        let (mut expected, expected_resolved_ids) =
            parse_session_transcript(epoch_input(&path)).unwrap();
        let page = MessageRange {
            limit: Some(100),
            ..MessageRange::default()
        };
        actual.initial_epoch = true;
        actual.apply_range(page.clone());
        expected.initial_epoch = true;
        expected.apply_range(page);

        assert_eq!(
            (
                serde_json::to_value(actual).unwrap(),
                resolved_ids,
                super::cached_tail_window_message_limit("fixture-session", &cache),
            ),
            (
                serde_json::to_value(expected).unwrap(),
                expected_resolved_ids,
                Some(100),
            ),
        );
    }

    #[rstest::rstest]
    #[case::no_epoch(None, true)]
    #[case::undecodable_epoch(Some("x"), false)]
    fn cached_tail_window_rebuild_policy(
        #[case] requested_epoch: Option<&str>,
        #[case] expected: bool,
    ) {
        assert_eq!(
            super::should_rebuild_tail_window(
                true,
                super::TranscriptRangeContext {
                    requested_epoch,
                    ..super::TranscriptRangeContext::default()
                },
                Some("append_requires_branch_reparse"),
                false,
            ),
            expected,
        );
    }

    #[rstest::rstest]
    fn cached_tail_window_rebuild_restores_pending_task_state_after_a_branch_switch() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.jsonl");
        let pending_task_ids = vec!["fixture-task-id".to_owned()];
        let mut transcript = indoc! {r#"
            {"type":"assistant","uuid":"fixture-task-call","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"fixture-task-use","name":"Bash","input":{"command":"fixture background task","run_in_background":true}}]}}
            {"type":"user","uuid":"fixture-task-started","parentUuid":"fixture-task-call","timestamp":"2026-01-01T00:00:02Z","toolUseResult":{"task_id":"fixture-task-id"},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-task-use","content":"Background task started (id: fixture-task-id)"}]}}
        "#}
        .trim()
        .to_owned();
        let mut parent_uuid = "fixture-task-started".to_owned();
        for index in 0..130 {
            let user_uuid = format!("fixture-user-{index}");
            let assistant_uuid = format!("fixture-assistant-{index}");
            let timestamp = format!("2026-01-01T00:{:02}:{:02}Z", index / 60, index % 60);
            transcript.push('\n');
            transcript.push_str(
                &serde_json::json!({
                    "type": "user",
                    "uuid": &user_uuid,
                    "parentUuid": &parent_uuid,
                    "timestamp": &timestamp,
                    "message": {"content": format!("Fixture request {index}.")},
                })
                .to_string(),
            );
            transcript.push('\n');
            transcript.push_str(
                &serde_json::json!({
                    "type": "assistant",
                    "uuid": &assistant_uuid,
                    "parentUuid": &user_uuid,
                    "timestamp": &timestamp,
                    "message": {"content": [{"type": "text", "text": format!("Fixture response {index}.")}]},
                })
                .to_string(),
            );
            parent_uuid = assistant_uuid;
        }
        transcript.push('\n');
        std::fs::write(&path, transcript).unwrap();

        let cache = MessageCache::default();
        let initial_fingerprint = transcript_fingerprint(Engine::Claude, Some(&path))
            .unwrap()
            .unwrap();
        super::tail_window::parse_initial_or_full_for_range(
            "fixture-session",
            epoch_input(&path),
            Some(100),
            &pending_task_ids,
            Some(initial_fingerprint),
            &cache,
            super::TranscriptRangeContext::default(),
        )
        .unwrap();

        let mut transcript = std::fs::read_to_string(&path).unwrap();
        transcript.push_str(
            &serde_json::json!({
                "type": "user",
                "uuid": "fixture-rewind-request",
                "parentUuid": "fixture-assistant-20",
                "timestamp": "2026-01-01T02:00:00Z",
                "message": {"content": "Continue from an earlier request."},
            })
            .to_string(),
        );
        transcript.push('\n');
        transcript.push_str(
            &serde_json::json!({
                "type": "assistant",
                "uuid": "fixture-rewind-response",
                "parentUuid": "fixture-rewind-request",
                "timestamp": "2026-01-01T02:00:01Z",
                "message": {"content": [{"type": "text", "text": "The earlier branch continues."}]},
            })
            .to_string(),
        );
        transcript.push('\n');
        std::fs::write(&path, transcript).unwrap();

        let fingerprint = transcript_fingerprint(Engine::Claude, Some(&path))
            .unwrap()
            .unwrap();
        let (actual, _, _) = super::tail_window::parse_initial_or_full_for_range(
            "fixture-session",
            epoch_input(&path),
            None,
            &pending_task_ids,
            Some(fingerprint),
            &cache,
            super::TranscriptRangeContext::default(),
        )
        .unwrap();
        let (expected, _) = parse_session_transcript(epoch_input(&path)).unwrap();

        assert_eq!(
            (
                actual.background_tasks,
                super::cached_tail_window_message_limit("fixture-session", &cache),
            ),
            (expected.background_tasks, Some(100)),
        );
    }

    #[rstest::fixture]
    fn epoch_transcript() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.jsonl");
        let mut records = Vec::new();
        let mut parent_uuid = None;
        for index in 0..180 {
            let user_uuid = format!("fixture-user-{index}");
            let assistant_uuid = format!("fixture-assistant-{index}");
            let parent = parent_uuid
                .as_deref()
                .map_or("null".to_owned(), |parent| format!("\"{parent}\""));
            records.push(format!(
                r#"{{"type":"user","uuid":"{user_uuid}","parentUuid":{parent},"timestamp":"2026-01-01T00:{:02}:{:02}Z","message":{{"content":"Fixture request {index}."}}}}"#,
                index / 60,
                index % 60,
            ));
            records.push(format!(
                r#"{{"type":"assistant","uuid":"{assistant_uuid}","parentUuid":"{user_uuid}","timestamp":"2026-01-01T00:{:02}:{:02}Z","message":{{"content":[{{"type":"text","text":"Fixture response {index}."}}]}}}}"#,
                index / 60,
                index % 60,
            ));
            parent_uuid = Some(assistant_uuid);
        }
        std::fs::write(&path, records.join("\n")).unwrap();
        directory
    }

    fn epoch_input(path: &std::path::Path) -> TranscriptInput<'_> {
        TranscriptInput {
            path: Some(path),
            engine: Engine::Claude,
            additions: ParserAdditions {
                session_id: "fixture-session",
                parent: None,
                pending_tool: None,
                user_marker: None,
                queued_prompts: &[],
                peer_out_messages: &[],
                proposals: &[],
                todos: &[],
                additional_cards: &[],
            },
        }
    }

    fn epoch_range(epoch: &str) -> MessageRange {
        MessageRange {
            from: Some(0),
            epoch: Some(epoch.to_owned()),
            ..MessageRange::default()
        }
    }

    fn expected_epoch_response(
        path: &std::path::Path,
        epoch: &str,
    ) -> (ConversationResponse, Vec<i64>) {
        let (mut conversation, resolved_ids) = parse_session_transcript(epoch_input(path)).unwrap();
        conversation.apply_range(epoch_range(epoch));
        (conversation, resolved_ids)
    }

    fn parse_epoch_refresh(
        path: &std::path::Path,
        epoch: &str,
        cache: &MessageCache,
    ) -> (ConversationResponse, Vec<i64>) {
        let fingerprint = transcript_fingerprint(Engine::Claude, Some(path))
            .unwrap()
            .unwrap();
        let (mut conversation, resolved_ids, _) =
            super::tail_window::parse_initial_or_full_for_range(
                "fixture-session",
                epoch_input(path),
                None,
                &[],
                Some(fingerprint),
                cache,
                super::TranscriptRangeContext {
                    requested_epoch: Some(epoch),
                    ..super::TranscriptRangeContext::default()
                },
            )
            .unwrap();
        conversation.apply_range(epoch_range(epoch));
        (conversation, resolved_ids)
    }

    #[rstest::rstest]
    fn cold_epoch_refresh_rebuilds_a_tail_window_parser(epoch_transcript: tempfile::TempDir) {
        let path = epoch_transcript.path().join("fixture.jsonl");
        let mut full = parse_session_transcript(epoch_input(&path)).unwrap().0;
        full.initial_epoch = true;
        full.apply_range(MessageRange {
            limit: Some(100),
            ..MessageRange::default()
        });
        let epoch = full.epoch.clone().unwrap();
        let cache = MessageCache::default();
        let actual = parse_epoch_refresh(&path, &epoch, &cache);
        let expected = expected_epoch_response(&path, &epoch);

        assert_eq!(
            (
                serde_json::to_value(&actual.0).unwrap(),
                actual.1,
                super::cached_tail_window_message_limit("fixture-session", &cache),
            ),
            (
                serde_json::to_value(expected.0).unwrap(),
                expected.1,
                Some(100),
            ),
        );
    }

    #[rstest::rstest]
    fn cold_epoch_refresh_falls_back_when_anchor_is_outside_the_tail(
        epoch_transcript: tempfile::TempDir,
    ) {
        let path = epoch_transcript.path().join("fixture.jsonl");
        let (full, _) = parse_session_transcript(epoch_input(&path)).unwrap();
        let mut initial_page = ConversationResponse {
            messages: full.messages[..100].to_vec(),
            initial_epoch: true,
            ..ConversationResponse::default()
        };
        initial_page.update_metadata();
        initial_page.apply_range(MessageRange {
            limit: Some(100),
            ..MessageRange::default()
        });
        let epoch = initial_page.epoch.clone().unwrap();
        let cache = MessageCache::default();
        let actual = parse_epoch_refresh(&path, &epoch, &cache);
        let expected = expected_epoch_response(&path, &epoch);

        assert_eq!(
            (
                serde_json::to_value(&actual.0).unwrap(),
                actual.1,
                super::cached_tail_window_message_limit("fixture-session", &cache),
            ),
            (serde_json::to_value(expected.0).unwrap(), expected.1, None,),
        );
    }

    #[rstest::rstest]
    fn undecodable_epoch_refresh_skips_tail_window_rebuild(epoch_transcript: tempfile::TempDir) {
        let path = epoch_transcript.path().join("fixture.jsonl");
        let cache = MessageCache::default();
        let actual = parse_epoch_refresh(&path, "x", &cache);
        let expected = expected_epoch_response(&path, "x");

        assert_eq!(
            (
                serde_json::to_value(&actual.0).unwrap(),
                actual.1,
                super::cached_tail_window_message_limit("fixture-session", &cache),
            ),
            (serde_json::to_value(expected.0).unwrap(), expected.1, None,),
        );
    }
}
