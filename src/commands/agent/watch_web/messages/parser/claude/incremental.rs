use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{self, BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::super::parser::jsonl::JsonlLineBuffer;
use super::state::{ClaudeParserState, is_attachment_record, record_parent_uuid};
use super::transcript_branch::{
    BranchMembership, TranscriptNode, active_record_uuids, mark_active_branch,
};
use super::user_input::is_tool_result_only_user_record;
use super::{ParsedTranscript, ParserAdditions};
use chrono::{DateTime, Utc};
use serde_json::Value;

mod continuation;
mod tail_window;
mod window;

use self::continuation::{AppendDecision, AppendHistory, classify_append};

#[derive(Clone)]
pub(in crate::commands::agent::watch_web::messages) struct IncrementalClaudeParser {
    state: ClaudeParserState,
    lines: JsonlLineBuffer,
    image_directory: Option<PathBuf>,
    pending_is_initial: bool,
    append_history: AppendHistory,
    tail_window_start: Option<DateTime<Utc>>,
    tail_window_start_offset: Option<u64>,
    active_record_cache: RefCell<Option<ActiveRecordCache>>,
}

#[derive(Clone)]
struct ActiveRecordCache {
    latest_record_uuid: Option<String>,
    active_record_uuids: Arc<HashSet<String>>,
}

impl IncrementalClaudeParser {
    pub(in crate::commands::agent::watch_web::messages) fn new(
        mut reader: impl BufRead + Seek,
        image_directory: Option<&Path>,
    ) -> io::Result<Self> {
        let mut graph_lines = JsonlLineBuffer::default();
        let mut transcript_nodes = std::collections::BTreeMap::new();
        let mut forward_parent_uuids = std::collections::HashSet::new();
        let mut latest_complete_uuid = None;
        let mut graph_latest_uuid = None;
        let mut record_order = 0;
        graph_lines.read_from(&mut reader, |line| {
            if let Some(record) = parse_record(line) {
                let parent_uuid = record_parent_uuid(&record);
                if let Some(parent_uuid) = parent_uuid.as_deref()
                    && !transcript_nodes.contains_key(parent_uuid)
                {
                    forward_parent_uuids.insert(parent_uuid.to_owned());
                }
                if !is_attachment_record(&record) {
                    latest_complete_uuid = record
                        .get("uuid")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or(latest_complete_uuid.clone());
                }
                if let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
                    graph_latest_uuid = Some(uuid.to_owned());
                    transcript_nodes.insert(
                        uuid.to_owned(),
                        TranscriptNode::from_record(&record, parent_uuid, record_order),
                    );
                    record_order += 1;
                }
            }
        })?;
        let pending_record = parse_record(graph_lines.pending());
        reader.seek(SeekFrom::Start(0))?;
        let mut state = ClaudeParserState::new();
        state.transcript_nodes = transcript_nodes;
        state.forward_parent_uuids = forward_parent_uuids;
        state.rebuild_transcript_children();
        activate_pending_record(
            &mut state,
            pending_record.as_ref(),
            &[],
            graph_latest_uuid,
            latest_complete_uuid,
        );
        let mut complete_lines = JsonlLineBuffer::default();
        let mut append_history = AppendHistory::default();
        complete_lines.read_from(&mut reader, |line| {
            if let Some(record) = parse_record(line) {
                ingest_record(
                    &mut state,
                    &mut append_history,
                    record,
                    Utc::now(),
                    RecordOrigin::Initial,
                );
            }
        })?;

        Ok(Self {
            state,
            lines: graph_lines,
            image_directory: image_directory.map(Path::to_path_buf),
            pending_is_initial: true,
            append_history,
            tail_window_start: None,
            tail_window_start_offset: None,
            active_record_cache: RefCell::default(),
        })
    }

    pub(in crate::commands::agent::watch_web::messages) fn new_tail_window_with_length(
        reader: impl BufRead + Seek,
        image_directory: Option<&Path>,
        message_limit: usize,
        pending_task_ids: &[String],
        file_length: u64,
    ) -> io::Result<Option<Self>> {
        window::new_tail_window_with_length(
            reader,
            image_directory,
            message_limit,
            pending_task_ids,
            file_length,
        )
    }

    pub(super) fn from_tail_window_records(
        records: Vec<Value>,
        active_ancestor_uuids: Vec<String>,
        image_directory: Option<&Path>,
        pending_line: Vec<u8>,
    ) -> Self {
        let mut state = ClaudeParserState::new();
        let mut seen_uuids = active_ancestor_uuids
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let mut latest_complete_uuid = None;
        let mut graph_latest_uuid = None;
        for record in &records {
            let parent_uuid = record_parent_uuid(record);
            if parent_uuid
                .as_deref()
                .is_some_and(|parent_uuid| !seen_uuids.contains(parent_uuid))
                && let Some(parent_uuid) = parent_uuid
            {
                state.forward_parent_uuids.insert(parent_uuid);
            }
            if !is_attachment_record(record) {
                latest_complete_uuid = record
                    .get("uuid")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or(latest_complete_uuid);
            }
            if let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
                graph_latest_uuid = Some(uuid.to_owned());
                seen_uuids.insert(uuid.to_owned());
                let record_order = state.transcript_nodes.len() as u64;
                state.transcript_nodes.insert(
                    uuid.to_owned(),
                    TranscriptNode::from_record(record, record_parent_uuid(record), record_order),
                );
            }
        }
        state.rebuild_transcript_children();
        let pending_record = parse_record(&pending_line);
        activate_pending_record(
            &mut state,
            pending_record.as_ref(),
            &active_ancestor_uuids,
            graph_latest_uuid,
            latest_complete_uuid,
        );
        let mut append_history = AppendHistory::default();
        for record in records {
            ingest_record(
                &mut state,
                &mut append_history,
                record,
                Utc::now(),
                RecordOrigin::Initial,
            );
        }

        Self {
            state,
            lines: JsonlLineBuffer::with_pending(pending_line),
            image_directory: image_directory.map(Path::to_path_buf),
            pending_is_initial: true,
            append_history,
            tail_window_start: None,
            tail_window_start_offset: None,
            active_record_cache: RefCell::default(),
        }
    }

    pub(in crate::commands::agent::watch_web::messages) fn tail_window_start(
        &self,
    ) -> Option<DateTime<Utc>> {
        self.tail_window_start
    }

    pub(in crate::commands::agent::watch_web::messages) fn tail_window_start_offset(
        &self,
    ) -> Option<u64> {
        self.tail_window_start_offset
    }

    pub(in crate::commands::agent::watch_web::messages) fn resolve_queued_prompt_ids_before_window(
        &self,
        reader: &mut (impl Read + Seek),
        prefix_length: u64,
        queued_prompts: &[crate::commands::agent::store::WatchWebQueuedPrompt],
        user_marker: Option<&str>,
    ) -> io::Result<Option<Vec<i64>>> {
        let (resolved_ids, complete) = tail_window::find_queued_prompt_deliveries(
            reader,
            prefix_length,
            queued_prompts,
            user_marker,
            &self.state.transcript_nodes,
        )?;
        Ok(complete.then_some(resolved_ids))
    }

    pub(in crate::commands::agent::watch_web::messages) fn append_bytes(
        &mut self,
        suffix: &[u8],
        now: DateTime<Utc>,
    ) -> io::Result<IncrementalAppend> {
        if suffix.is_empty() {
            return Ok(IncrementalAppend::Applied);
        }
        self.active_record_cache.get_mut().take();
        let mut restart_required = false;
        let state = &mut self.state;
        let append_history = &mut self.append_history;
        let lines = &mut self.lines;
        lines.feed(suffix, |line| {
            if restart_required {
                return;
            }
            let Some(record) = parse_record(line) else {
                return;
            };
            if !ingest_record(state, append_history, record, now, RecordOrigin::Appended) {
                restart_required = true;
            }
        });
        if !restart_required
            && let Some(record) = parse_record(lines.pending())
            && classify_append(state, append_history, &record).is_none()
        {
            restart_required = true;
        }
        if restart_required {
            return Ok(IncrementalAppend::RestartRequired);
        }
        self.pending_is_initial = false;
        Ok(IncrementalAppend::Applied)
    }

    pub(in crate::commands::agent::watch_web::messages) fn render_with(
        &self,
        now: DateTime<Utc>,
        additions: ParserAdditions<'_>,
    ) -> io::Result<ParsedTranscript> {
        let pending = self.lines.pending();
        if let Some(record) = parse_record(pending) {
            let mut state = self.state.clone();
            if !ingest_record(
                &mut state,
                &mut self.append_history.clone(),
                record,
                now,
                RecordOrigin::Pending {
                    is_initial: self.pending_is_initial,
                },
            ) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Claude transcript tail forks from the cached active branch",
                ));
            }
            let active_record_uuids = self.tail_window_start_offset.is_none().then(|| {
                Arc::new(active_record_uuids(
                    &state.transcript_nodes,
                    state.latest_record_uuid.as_deref(),
                ))
            });
            return Ok(super::render::render(
                &state,
                self.image_directory.as_deref(),
                now,
                additions,
                active_record_uuids.as_deref(),
            ));
        }
        let active_record_uuids = self
            .tail_window_start_offset
            .is_none()
            .then(|| self.cached_active_record_uuids(&self.state));
        Ok(super::render::render(
            &self.state,
            self.image_directory.as_deref(),
            now,
            additions,
            active_record_uuids.as_deref(),
        ))
    }

    fn cached_active_record_uuids(&self, state: &ClaudeParserState) -> Arc<HashSet<String>> {
        let latest_record_uuid = state.latest_record_uuid.clone();
        let cached = self
            .active_record_cache
            .borrow()
            .as_ref()
            .filter(|cache| cache.latest_record_uuid == latest_record_uuid)
            .map(|cache| Arc::clone(&cache.active_record_uuids));
        if let Some(cached) = cached {
            return cached;
        }

        let active_record_uuids = Arc::new(active_record_uuids(
            &state.transcript_nodes,
            latest_record_uuid.as_deref(),
        ));
        *self.active_record_cache.borrow_mut() = Some(ActiveRecordCache {
            latest_record_uuid,
            active_record_uuids: Arc::clone(&active_record_uuids),
        });
        active_record_uuids
    }
}

fn prepare_pending_render_state(state: &mut ClaudeParserState, record: &Value) {
    let Some(uuid) = record.get("uuid").and_then(Value::as_str) else {
        return;
    };
    if !state.transcript_nodes.contains_key(uuid) {
        let parent_uuid = record_parent_uuid(record);
        let record_order = state.transcript_nodes.len() as u64;
        state.transcript_nodes.insert(
            uuid.to_owned(),
            TranscriptNode::from_record(record, parent_uuid.clone(), record_order),
        );
        state.index_transcript_child(parent_uuid.as_deref(), uuid);
    }
    if !is_tool_result_only_user_record(record) {
        mark_active_branch(&mut state.transcript_nodes, Some(uuid));
        state.latest_record_uuid = Some(uuid.to_owned());
        state.latest_uuid = Some(uuid.to_owned());
    }
}

fn activate_pending_record(
    state: &mut ClaudeParserState,
    pending_record: Option<&Value>,
    active_ancestor_uuids: &[String],
    fallback_latest_record_uuid: Option<String>,
    fallback_latest_uuid: Option<String>,
) {
    let pending_uuid = pending_record
        .and_then(|record| record.get("uuid").and_then(Value::as_str))
        .map(str::to_owned);
    let pending_node_inserted =
        if let (Some(record), Some(uuid)) = (pending_record, pending_uuid.as_deref()) {
            if state.transcript_nodes.contains_key(uuid) {
                false
            } else {
                let record_order = state.transcript_nodes.len() as u64;
                state.transcript_nodes.insert(
                    uuid.to_owned(),
                    TranscriptNode::from_record(record, record_parent_uuid(record), record_order),
                );
                true
            }
        } else {
            false
        };
    for uuid in active_ancestor_uuids {
        state
            .transcript_nodes
            .entry(uuid.clone())
            .and_modify(|node| node.membership = BranchMembership::Active)
            .or_insert(TranscriptNode {
                record_order: 0,
                parent_uuid: None,
                membership: BranchMembership::Active,
                has_tool_use: false,
                tool_use_ids: Vec::new(),
                tool_result_ids: Vec::new(),
                is_tool_result_only: false,
                is_task_notification: false,
                was_valid_tool_result: false,
                processed: false,
            });
    }
    // A pending first result can arrive while a sibling tool call is still running.
    let pending_parallel_tool_use =
        pending_record.and_then(|record| state.parallel_pending_tool_use_uuid(record));
    mark_active_branch(
        &mut state.transcript_nodes,
        pending_parallel_tool_use
            .as_deref()
            .or(pending_uuid.as_deref())
            .or(fallback_latest_record_uuid.as_deref())
            .or(fallback_latest_uuid.as_deref()),
    );
    if pending_node_inserted && let Some(uuid) = pending_uuid.as_deref() {
        state.transcript_nodes.remove(uuid);
    }
    state.latest_record_uuid = if pending_uuid.is_some() {
        pending_record.and_then(record_parent_uuid)
    } else {
        fallback_latest_record_uuid
    };
    state.latest_uuid = if pending_uuid.is_some() {
        pending_record.and_then(record_parent_uuid)
    } else {
        fallback_latest_uuid
    };
}

#[derive(Clone, Copy)]
enum RecordOrigin {
    Initial,
    Appended,
    Pending { is_initial: bool },
}

fn ingest_record(
    state: &mut ClaudeParserState,
    append_history: &mut AppendHistory,
    record: Value,
    now: DateTime<Utc>,
    origin: RecordOrigin,
) -> bool {
    let append_decision = if matches!(
        origin,
        RecordOrigin::Appended | RecordOrigin::Pending { is_initial: false }
    ) {
        let Some(decision) = classify_append(state, append_history, &record) else {
            return false;
        };
        Some(decision)
    } else {
        None
    };
    if matches!(origin, RecordOrigin::Pending { .. }) {
        prepare_pending_render_state(state, &record);
    }
    if !matches!(origin, RecordOrigin::Initial)
        && let Some(uuid) = record.get("uuid").and_then(Value::as_str)
    {
        let parent_uuid = record_parent_uuid(&record);
        let record_order = state.transcript_nodes.len() as u64;
        state.transcript_nodes.insert(
            uuid.to_owned(),
            TranscriptNode::from_record(&record, parent_uuid.clone(), record_order),
        );
        state.index_transcript_child(parent_uuid.as_deref(), uuid);
        mark_active_branch(&mut state.transcript_nodes, Some(uuid));
        if matches!(origin, RecordOrigin::Appended) {
            state.latest_record_uuid = Some(uuid.to_owned());
            state.latest_uuid = Some(uuid.to_owned());
        }
    }
    if !matches!(origin, RecordOrigin::Pending { .. }) {
        append_history.observe(&record);
    }
    state.process_record(record, now);
    if matches!(origin, RecordOrigin::Appended) {
        match append_decision {
            Some(AppendDecision::SimpleTail | AppendDecision::ParallelResult) => {}
            None => return false,
        }
    }
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::commands::agent::watch_web::messages) enum IncrementalAppend {
    Applied,
    RestartRequired,
}

fn parse_record(line: &[u8]) -> Option<Value> {
    let line = std::str::from_utf8(line).ok()?;
    serde_json::from_str(line).ok()
}

#[cfg(test)]
mod tests;
