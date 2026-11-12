use std::io::{BufRead, Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::super::super::ParentSession;
use super::transcript_branch::{BranchMembership, TranscriptNode, mark_active_branch};
use super::{
    ClaudeParserState, ParseContext, ParsedTranscript, ParserAdditions, TranscriptParserContext,
};

#[derive(Clone)]
pub(in crate::commands::agent::watch_web::messages) struct IncrementalClaudeParser {
    state: ClaudeParserState,
    context: TranscriptParserContext,
    pending_line: Vec<u8>,
    tail_from_initial_scan: bool,
    compressed_chunks: Vec<Vec<u8>>,
}

pub(in crate::commands::agent::watch_web::messages) enum IncrementalClaudeSnapshot {
    Parsed(ParsedTranscript),
    RestartRequired,
}

impl IncrementalClaudeParser {
    pub(in crate::commands::agent::watch_web::messages) fn new(
        reader: impl BufRead + Seek,
        parent: Option<&ParentSession>,
        pending_tool: Option<&str>,
        user_marker: Option<&str>,
        now: DateTime<Utc>,
        additions: ParserAdditions<'_>,
    ) -> std::io::Result<Self> {
        let image_directory =
            crate::commands::agent::watch_web::image_attachments::storage_directory().ok();
        Self::from_reader(
            reader,
            parent,
            pending_tool,
            user_marker,
            now,
            ParseContext {
                image_directory: image_directory.as_deref(),
                queued_prompts: additions.queued_prompts,
                peer_out_messages: additions.peer_out_messages,
                decisions: additions.decisions,
                proposals: additions.proposals,
                todos: additions.todos,
            },
        )
    }

    pub(super) fn from_reader(
        mut reader: impl BufRead + Seek,
        parent: Option<&ParentSession>,
        pending_tool: Option<&str>,
        user_marker: Option<&str>,
        now: DateTime<Utc>,
        context: ParseContext<'_>,
    ) -> std::io::Result<Self> {
        let parser_context = TranscriptParserContext {
            parent: parent.cloned(),
            pending_tool: pending_tool.map(str::to_owned),
            user_marker: user_marker.map(str::to_owned),
            image_directory: context.image_directory.map(Path::to_path_buf),
            decisions: context.decisions.to_vec(),
            queued_prompts: context.queued_prompts.to_vec(),
            peer_out_messages: context.peer_out_messages.to_vec(),
            proposals: context.proposals.to_vec(),
            todos: context.todos.to_vec(),
        };
        let mut state = ClaudeParserState::new(&context);
        let mut latest_complete_uuid = None;
        let mut latest_uuid = None;
        let mut pending_line = Vec::new();
        let mut line = Vec::new();
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), 1)?;
        loop {
            line.clear();
            let read = reader.read_until(b'\n', &mut line)?;
            if read == 0 {
                break;
            }
            encoder.write_all(&line)?;
            let is_complete = line.ends_with(b"\n");
            if let Ok(line_text) = std::str::from_utf8(&line)
                && let Ok(record) = serde_json::from_str::<Value>(line_text)
            {
                if is_complete {
                    state.queue.observe(&record);
                    latest_complete_uuid = record
                        .get("uuid")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or(latest_complete_uuid);
                }
                if let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
                    latest_uuid = Some(uuid.to_owned());
                    state.transcript_nodes.insert(
                        uuid.to_owned(),
                        TranscriptNode {
                            parent_uuid: record_parent_uuid(&record),
                            membership: BranchMembership::Unknown,
                        },
                    );
                }
            }
            if !is_complete {
                pending_line = line.clone();
                break;
            }
        }
        mark_active_branch(&mut state.transcript_nodes, latest_uuid.as_deref());
        state.latest_uuid = latest_complete_uuid;
        reader.seek(SeekFrom::Start(0))?;
        for_each_complete_jsonl_record(&mut reader, |record| {
            state.process_record(record, &parser_context, now);
        })?;
        let compressed_transcript = encoder.finish()?;
        Ok(Self {
            state,
            context: parser_context,
            pending_line,
            tail_from_initial_scan: true,
            compressed_chunks: vec![compressed_transcript],
        })
    }

    pub(in crate::commands::agent::watch_web::messages) fn snapshot(
        &self,
        now: DateTime<Utc>,
    ) -> std::io::Result<IncrementalClaudeSnapshot> {
        self.snapshot_from_context(now, &self.context)
    }

    pub(in crate::commands::agent::watch_web::messages) fn snapshot_with(
        &mut self,
        now: DateTime<Utc>,
        parent: Option<&ParentSession>,
        pending_tool: Option<&str>,
        user_marker: Option<&str>,
        additions: ParserAdditions<'_>,
    ) -> std::io::Result<IncrementalClaudeSnapshot> {
        let context = TranscriptParserContext {
            parent: parent.cloned(),
            pending_tool: pending_tool.map(str::to_owned),
            user_marker: user_marker.map(str::to_owned),
            image_directory: self.context.image_directory.clone(),
            decisions: additions.decisions.to_vec(),
            queued_prompts: additions.queued_prompts.to_vec(),
            peer_out_messages: additions.peer_out_messages.to_vec(),
            proposals: additions.proposals.to_vec(),
            todos: additions.todos.to_vec(),
        };
        if context == self.context {
            return self.snapshot_from_context(now, &context);
        }

        let mut transcript = Vec::new();
        for chunk in &self.compressed_chunks {
            let mut decoder = zstd::stream::read::Decoder::new(Cursor::new(chunk))?;
            decoder.read_to_end(&mut transcript)?;
        }
        let parse_context = ParseContext {
            image_directory: context.image_directory.as_deref(),
            queued_prompts: &context.queued_prompts,
            peer_out_messages: &context.peer_out_messages,
            decisions: &context.decisions,
            proposals: &context.proposals,
            todos: &context.todos,
        };
        let rebased = Self::from_reader(
            Cursor::new(transcript),
            context.parent.as_ref(),
            context.pending_tool.as_deref(),
            context.user_marker.as_deref(),
            now,
            parse_context,
        )?;
        let snapshot = rebased.snapshot_from_context(now, &context)?;
        if matches!(snapshot, IncrementalClaudeSnapshot::Parsed(_)) {
            *self = rebased;
        }
        Ok(snapshot)
    }

    fn snapshot_from_context(
        &self,
        now: DateTime<Utc>,
        context: &TranscriptParserContext,
    ) -> std::io::Result<IncrementalClaudeSnapshot> {
        let mut state = self.state.clone();
        if !self.pending_line.is_empty()
            && let Ok(line) = std::str::from_utf8(&self.pending_line)
            && let Ok(record) = serde_json::from_str::<Value>(line)
        {
            if !self.tail_from_initial_scan && !is_active_branch_continuation(&state, &record) {
                return Ok(IncrementalClaudeSnapshot::RestartRequired);
            }
            state.queue.observe(&record);
            if let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
                state.transcript_nodes.insert(
                    uuid.to_owned(),
                    TranscriptNode {
                        parent_uuid: record_parent_uuid(&record),
                        membership: BranchMembership::Unknown,
                    },
                );
                mark_active_branch(&mut state.transcript_nodes, Some(uuid));
            }
            state.process_record(record, context, now);
        }
        Ok(IncrementalClaudeSnapshot::Parsed(
            state.into_parsed(context, now),
        ))
    }

    pub(in crate::commands::agent::watch_web::messages) fn append_bytes(
        &mut self,
        suffix: &[u8],
        now: DateTime<Utc>,
    ) -> std::io::Result<IncrementalAppend> {
        if !suffix.is_empty() {
            self.compressed_chunks
                .push(zstd::stream::encode_all(Cursor::new(suffix), 1)?);
        }
        let mut bytes = std::mem::take(&mut self.pending_line);
        bytes.extend_from_slice(suffix);
        self.tail_from_initial_scan = false;
        let mut start = 0;
        while let Some(relative_end) = bytes[start..].iter().position(|byte| *byte == b'\n') {
            let end = start + relative_end + 1;
            let line = &bytes[start..end];
            if let Ok(line_text) = std::str::from_utf8(line)
                && let Ok(record) = serde_json::from_str::<Value>(line_text)
            {
                if !is_active_branch_continuation(&self.state, &record) {
                    self.pending_line = bytes[start..].to_vec();
                    return Ok(IncrementalAppend::RestartRequired);
                }
                self.state.queue.observe(&record);
                if let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
                    self.state.transcript_nodes.insert(
                        uuid.to_owned(),
                        TranscriptNode {
                            parent_uuid: record_parent_uuid(&record),
                            membership: BranchMembership::Unknown,
                        },
                    );
                    mark_active_branch(&mut self.state.transcript_nodes, Some(uuid));
                    self.state.latest_uuid = Some(uuid.to_owned());
                }
                self.state.process_record(record, &self.context, now);
            }
            start = end;
        }
        self.pending_line = bytes[start..].to_vec();
        Ok(IncrementalAppend::Applied)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(in crate::commands::agent::watch_web::messages) enum IncrementalAppend {
    Applied,
    RestartRequired,
}

fn for_each_complete_jsonl_record(
    reader: &mut impl BufRead,
    mut visit: impl FnMut(Value),
) -> std::io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Err(error) => return Err(error),
            Ok(_) if !line.ends_with(b"\n") => break,
            Ok(_) => {}
        }
        let Ok(line) = std::str::from_utf8(&line) else {
            continue;
        };
        if let Ok(record) = serde_json::from_str(line) {
            visit(record);
        }
    }
    Ok(())
}

fn record_parent_uuid(record: &Value) -> Option<String> {
    match record.get("parentUuid") {
        Some(Value::String(parent)) => Some(parent.clone()),
        Some(Value::Null) => match record.get("logicalParentUuid") {
            None | Some(Value::Null) => None,
            Some(Value::String(parent)) => Some(parent.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn is_active_branch_continuation(state: &ClaudeParserState, record: &Value) -> bool {
    if record.get("uuid").and_then(Value::as_str).is_none() {
        return true;
    }
    let parent_uuid = record_parent_uuid(record);
    parent_uuid.as_deref() == state.latest_uuid.as_deref()
}

#[cfg(test)]
mod tests;
