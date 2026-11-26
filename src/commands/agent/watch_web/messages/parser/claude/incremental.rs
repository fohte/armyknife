use std::io::{self, BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::super::super::parser::jsonl::JsonlLineBuffer;
use super::state::{ClaudeParserState, is_attachment_record, record_parent_uuid};
use super::transcript_branch::{BranchMembership, TranscriptNode, mark_active_branch};
use super::user_input::is_tool_result_only_user_record;
use super::{ParsedTranscript, ParserAdditions};
use chrono::{DateTime, Utc};
use serde_json::Value;

mod tail_window;
mod window;

#[derive(Clone)]
pub(in crate::commands::agent::watch_web::messages) struct IncrementalClaudeParser {
    state: ClaudeParserState,
    lines: JsonlLineBuffer,
    image_directory: Option<PathBuf>,
    pending_is_initial: bool,
    tail_window_start: Option<DateTime<Utc>>,
    tail_window_start_offset: Option<u64>,
}

impl IncrementalClaudeParser {
    pub(in crate::commands::agent::watch_web::messages) fn new(
        mut reader: impl BufRead + Seek,
        image_directory: Option<&Path>,
    ) -> io::Result<Self> {
        let mut graph_lines = JsonlLineBuffer::default();
        let mut transcript_nodes = std::collections::BTreeMap::new();
        let mut latest_complete_uuid = None;
        let mut graph_latest_uuid = None;
        graph_lines.read_from(&mut reader, |line| {
            if let Some(record) = parse_record(line) {
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
                        TranscriptNode {
                            parent_uuid: record_parent_uuid(&record),
                            membership: BranchMembership::Unknown,
                        },
                    );
                }
            }
        })?;
        let pending_record = parse_record(graph_lines.pending());
        reader.seek(SeekFrom::Start(0))?;
        let mut state = ClaudeParserState::new();
        state.transcript_nodes = transcript_nodes;
        state.rebuild_transcript_children();
        activate_pending_record(
            &mut state,
            pending_record.as_ref(),
            &[],
            graph_latest_uuid,
            latest_complete_uuid,
        );
        let mut complete_lines = JsonlLineBuffer::default();
        complete_lines.read_from(&mut reader, |line| {
            if let Some(record) = parse_record(line) {
                ingest_record(&mut state, record, Utc::now(), RecordOrigin::Initial);
            }
        })?;

        Ok(Self {
            state,
            lines: graph_lines,
            image_directory: image_directory.map(Path::to_path_buf),
            pending_is_initial: true,
            tail_window_start: None,
            tail_window_start_offset: None,
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
        let mut latest_complete_uuid = None;
        let mut graph_latest_uuid = None;
        for record in &records {
            if !is_attachment_record(record) {
                latest_complete_uuid = record
                    .get("uuid")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or(latest_complete_uuid);
            }
            if let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
                graph_latest_uuid = Some(uuid.to_owned());
                state.transcript_nodes.insert(
                    uuid.to_owned(),
                    TranscriptNode {
                        parent_uuid: record_parent_uuid(record),
                        membership: BranchMembership::Unknown,
                    },
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
        for record in records {
            ingest_record(&mut state, record, Utc::now(), RecordOrigin::Initial);
        }

        Self {
            state,
            lines: JsonlLineBuffer::with_pending(pending_line),
            image_directory: image_directory.map(Path::to_path_buf),
            pending_is_initial: true,
            tail_window_start: None,
            tail_window_start_offset: None,
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
        let state = &mut self.state;
        let lines = &mut self.lines;
        lines.feed(suffix, |line| {
            if restart_required {
                return;
            }
            let Some(record) = parse_record(line) else {
                return;
            };
            if !ingest_record(state, record, now, RecordOrigin::Appended) {
                restart_required = true;
            }
        });
        if !restart_required
            && let Some(record) = parse_record(lines.pending())
            && !is_active_branch_continuation(state, &record)
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
            return Ok(super::render::render(
                &state,
                self.image_directory.as_deref(),
                now,
                additions,
            ));
        }
        Ok(super::render::render(
            &self.state,
            self.image_directory.as_deref(),
            now,
            additions,
        ))
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
                state.transcript_nodes.insert(
                    uuid.to_owned(),
                    TranscriptNode {
                        parent_uuid: record_parent_uuid(record),
                        membership: BranchMembership::Unknown,
                    },
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
                parent_uuid: None,
                membership: BranchMembership::Active,
            });
    }
    // The unfinished row selects the render branch but is ingested after it is completed.
    mark_active_branch(
        &mut state.transcript_nodes,
        pending_uuid
            .as_deref()
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
    record: Value,
    now: DateTime<Utc>,
    origin: RecordOrigin,
) -> bool {
    if matches!(
        origin,
        RecordOrigin::Appended | RecordOrigin::Pending { is_initial: false }
    ) && !is_active_branch_continuation(state, &record)
    {
        return false;
    }
    if !matches!(origin, RecordOrigin::Initial)
        && let Some(uuid) = record.get("uuid").and_then(Value::as_str)
    {
        let parent_uuid = record_parent_uuid(&record);
        let parallel_tool_result = matches!(origin, RecordOrigin::Appended)
            && parent_uuid.as_deref().is_some_and(|parent_uuid| {
                state.has_pending_tool_result_for_parent(&record, Some(parent_uuid))
                    && parent_uuid != state.latest_uuid.as_deref().unwrap_or_default()
            });
        state.transcript_nodes.insert(
            uuid.to_owned(),
            TranscriptNode {
                parent_uuid: parent_uuid.clone(),
                membership: BranchMembership::Unknown,
            },
        );
        state.index_transcript_child(parent_uuid.as_deref(), uuid);
        mark_active_branch(&mut state.transcript_nodes, Some(uuid));
        if matches!(origin, RecordOrigin::Appended) {
            state.latest_record_uuid = Some(uuid.to_owned());
        }
        if matches!(origin, RecordOrigin::Appended)
            && !parallel_tool_result
            && !is_attachment_record(&record)
        {
            state.latest_uuid = Some(uuid.to_owned());
        }
    }
    state.process_record(record, now);
    true
}

fn is_active_branch_continuation(state: &ClaudeParserState, record: &Value) -> bool {
    let Some(uuid) = record.get("uuid").and_then(Value::as_str) else {
        return true;
    };
    if state.transcript_nodes.contains_key(uuid) {
        return false;
    }
    let parent_uuid = record_parent_uuid(record);
    let is_tool_result_only = is_tool_result_only_user_record(record);
    let is_task_notification = super::background_tasks::task_notification_text(record).is_some();
    if is_tool_result_only && parent_uuid.is_none() {
        return false;
    }
    let Some(parent_uuid) = parent_uuid else {
        return state.latest_uuid.is_none();
    };
    let parent_is_active = state
        .transcript_nodes
        .get(&parent_uuid)
        .is_some_and(|node| {
            matches!(
                node.membership,
                BranchMembership::Active | BranchMembership::ReachesActive
            )
        });
    if !parent_is_active {
        return false;
    }

    if is_tool_result_only {
        let latest_record_is_compact_boundary_branch =
            state.latest_record_is_compact_boundary_branch();
        return !latest_record_is_compact_boundary_branch
            && state.has_pending_tool_result_for_parent(record, Some(parent_uuid.as_str()));
    }
    let parent_is_latest = state.latest_record_uuid.as_deref() == Some(parent_uuid.as_str());
    if is_task_notification {
        return parent_is_latest;
    }
    if record.get("type").and_then(Value::as_str) == Some("assistant") {
        return parent_is_latest
            || state.tool_result_parent.as_deref() == Some(parent_uuid.as_str());
    }
    parent_is_latest
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
