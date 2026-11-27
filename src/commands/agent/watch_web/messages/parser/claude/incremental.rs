use std::cell::RefCell;
use std::io::{self, BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::super::super::parser::jsonl::JsonlLineBuffer;
#[cfg(test)]
use super::ParsedTranscript;
use super::state::{ClaudeParserState, is_attachment_record, record_parent_uuid};
use super::transcript_branch::{BranchMembership, TranscriptNode, mark_active_branch};
use super::user_input::is_tool_result_only_user_record;
use chrono::{DateTime, Utc};
use serde_json::Value;

mod continuation;
mod ingest;
mod render;
mod tail_window;
mod window;

use self::continuation::{AppendHistory, classify_append};
use self::ingest::{RecordOrigin, ingest_record};
use self::render::ActiveRecordCache;

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

impl IncrementalClaudeParser {
    pub(in crate::commands::agent::watch_web::messages) fn new(
        mut reader: impl BufRead + Seek,
        image_directory: Option<&Path>,
    ) -> io::Result<Self> {
        let mut graph_lines = JsonlLineBuffer::default();
        let mut state = ClaudeParserState::new();
        let mut latest_complete_uuid = None;
        let mut graph_latest_uuid = None;
        let mut record_order = 0_u64;
        let mut root_count = 0_usize;
        graph_lines.read_from(&mut reader, |line| {
            if let Some(record) = parse_record(line) {
                state.has_compact_boundary_record |= is_compact_boundary(&record);
                let parent_uuid = record_parent_uuid(&record);
                if !is_attachment_record(&record) {
                    latest_complete_uuid = record
                        .get("uuid")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or(latest_complete_uuid.clone());
                }
                if let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
                    graph_latest_uuid = Some(uuid.to_owned());
                    root_count += usize::from(parent_uuid.is_none());
                    let had_transcript_forks = state.has_transcript_forks;
                    let is_self_parent = parent_uuid.as_deref() == Some(uuid);
                    let parent_node = parent_uuid
                        .as_deref()
                        .and_then(|parent_uuid| state.transcript_nodes.get(parent_uuid));
                    let mut parent_order = parent_node.map(|parent| parent.record_order as usize);
                    let parent_has_tool_use = parent_node.is_some_and(|parent| parent.has_tool_use);
                    if parent_uuid.is_some()
                        && parent_order.is_none()
                        && let Some(parent_uuid) = parent_uuid.as_deref()
                    {
                        state.forward_parent_uuids.insert(parent_uuid.to_owned());
                    }
                    let uuid_id = state.transcript_nodes.get(uuid).map_or_else(
                        || {
                            let uuid_id = state.next_uuid_id;
                            state.next_uuid_id += 1;
                            uuid_id
                        },
                        |node: &TranscriptNode| node.uuid_id,
                    );
                    let node = TranscriptNode::from_record(
                        &record,
                        parent_uuid.clone(),
                        uuid_id,
                        record_order,
                    );
                    let is_relevant_child = parent_uuid.is_some()
                        && parent_has_tool_use
                        && (node.has_tool_use || node.is_tool_result_only);
                    let duplicate = state
                        .transcript_nodes
                        .insert(uuid.to_owned(), node)
                        .is_some();
                    state.has_duplicate_uuids |= duplicate;
                    state.transcript_parent_child_counts.push(0);
                    if !state.has_duplicate_uuids {
                        if is_self_parent {
                            parent_order = state
                                .transcript_nodes
                                .get(uuid)
                                .and_then(|node| node.parent_uuid.as_deref())
                                .and_then(|parent_uuid| state.transcript_nodes.get(parent_uuid))
                                .map(|parent| parent.record_order as usize);
                        }
                        if let Some(parent_order) = parent_order
                            && let Some(child_count) =
                                state.transcript_parent_child_counts.get_mut(parent_order)
                        {
                            state.has_transcript_forks |= *child_count > 0;
                            *child_count = child_count.saturating_add(1).min(2);
                        }
                    }
                    if !had_transcript_forks && state.has_transcript_forks {
                        state.rebuild_transcript_children();
                    } else if had_transcript_forks
                        && is_relevant_child
                        && !duplicate
                        && let Some(parent_uuid) = parent_uuid.as_deref()
                    {
                        state
                            .transcript_children
                            .entry(parent_uuid.to_owned())
                            .or_default()
                            .push(uuid.to_owned());
                    }
                    record_order += 1;
                }
            }
        })?;
        let pending_record = parse_record(graph_lines.pending());
        reader.seek(SeekFrom::Start(0))?;
        let pending_record_creates_fork = pending_record
            .as_ref()
            .and_then(record_parent_uuid)
            .and_then(|parent_uuid| state.transcript_nodes.get(&parent_uuid))
            .and_then(|parent| {
                state
                    .transcript_parent_child_counts
                    .get(parent.record_order as usize)
            })
            .is_some_and(|child_count| *child_count > 0);
        let simple_graph = !state.has_transcript_forks
            && !pending_record_creates_fork
            && !state.has_duplicate_uuids
            && state.forward_parent_uuids.is_empty()
            && !state.has_compact_boundary_record
            && root_count <= 1;
        if state.has_duplicate_uuids
            || !state.forward_parent_uuids.is_empty()
            || pending_record_creates_fork
        {
            state.rebuild_transcript_children();
        }
        reader.seek(SeekFrom::Start(0))?;
        activate_pending_record(
            &mut state,
            pending_record.as_ref(),
            &[],
            graph_latest_uuid,
            latest_complete_uuid,
        );
        state.all_transcript_records_active = (pending_record.is_none() && simple_graph)
            || state
                .transcript_nodes
                .values()
                .all(|node| node.membership == BranchMembership::Active);
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
                    None,
                );
            }
        })?;
        append_history.finish_initial();
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
        let mut next_record_order = 0_u64;
        for uuid in &active_ancestor_uuids {
            if state.transcript_nodes.contains_key(uuid) {
                continue;
            }
            state.transcript_nodes.insert(
                uuid.clone(),
                TranscriptNode {
                    uuid_id: state.next_uuid_id,
                    record_order: next_record_order,
                    parent_uuid: None,
                    membership: BranchMembership::Unknown,
                    has_tool_use: false,
                    tool_use_ids: None,
                    tool_result_ids: None,
                    is_tool_result_only: false,
                    is_task_notification: false,
                },
            );
            state.next_uuid_id += 1;
            next_record_order += 1;
        }
        let mut latest_complete_uuid = None;
        let mut graph_latest_uuid = None;
        for record in &records {
            state.has_compact_boundary_record |= is_compact_boundary(record);
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
                let record_order = next_record_order;
                state.has_duplicate_uuids |= state.transcript_nodes.contains_key(uuid);
                let uuid_id = state.transcript_nodes.get(uuid).map_or_else(
                    || {
                        let uuid_id = state.next_uuid_id;
                        state.next_uuid_id += 1;
                        uuid_id
                    },
                    |node| node.uuid_id,
                );
                state.transcript_nodes.insert(
                    uuid.to_owned(),
                    TranscriptNode::from_record(
                        record,
                        record_parent_uuid(record),
                        uuid_id,
                        record_order,
                    ),
                );
                next_record_order += 1;
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
        state.all_transcript_records_active = state
            .transcript_nodes
            .values()
            .all(|node| node.membership == BranchMembership::Active);
        let mut append_history = AppendHistory::default();
        for record in records {
            ingest_record(
                &mut state,
                &mut append_history,
                record,
                Utc::now(),
                RecordOrigin::Initial,
                None,
            );
        }
        append_history.finish_initial();

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
        let mut restart_required = false;
        let mut added_ids = Vec::new();
        let track_active_cache =
            self.state.has_transcript_forks && self.active_record_cache.get_mut().is_some();
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
            if !ingest_record(
                state,
                append_history,
                record,
                now,
                RecordOrigin::Appended,
                track_active_cache.then_some(&mut added_ids),
            ) {
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
        self.extend_active_record_cache(added_ids);
        self.pending_is_initial = false;
        Ok(IncrementalAppend::Applied)
    }
}

fn prepare_pending_render_state(state: &mut ClaudeParserState, record: &Value) {
    let Some(uuid) = record.get("uuid").and_then(Value::as_str) else {
        return;
    };
    if !state.transcript_nodes.contains_key(uuid) {
        let parent_uuid = record_parent_uuid(record);
        let record_order = state.transcript_parent_child_counts.len() as u64;
        let uuid_id = state.next_uuid_id;
        state.transcript_nodes.insert(
            uuid.to_owned(),
            TranscriptNode::from_record(record, parent_uuid.clone(), uuid_id, record_order),
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
                let record_order = state.transcript_parent_child_counts.len() as u64;
                let uuid_id = state.next_uuid_id;
                state.transcript_nodes.insert(
                    uuid.to_owned(),
                    TranscriptNode::from_record(
                        record,
                        record_parent_uuid(record),
                        uuid_id,
                        record_order,
                    ),
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
                uuid_id: state.next_uuid_id,
                record_order: 0,
                parent_uuid: None,
                membership: BranchMembership::Active,
                has_tool_use: false,
                tool_use_ids: None,
                tool_result_ids: None,
                is_tool_result_only: false,
                is_task_notification: false,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::commands::agent::watch_web::messages) enum IncrementalAppend {
    Applied,
    RestartRequired,
}

fn parse_record(line: &[u8]) -> Option<Value> {
    let line = std::str::from_utf8(line).ok()?;
    serde_json::from_str(line).ok()
}

fn is_compact_boundary(record: &Value) -> bool {
    record.get("type").and_then(Value::as_str) == Some("system")
        && record.get("subtype").and_then(Value::as_str) == Some("compact_boundary")
}

#[cfg(test)]
mod tests;
