use std::io::{self, BufRead, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::super::super::parser::jsonl::JsonlLineBuffer;
use super::super::record_timestamp;
use super::state::{ClaudeParserState, is_attachment_record, record_parent_uuid};
use super::transcript_branch::{BranchMembership, TranscriptNode, mark_active_branch};
use super::user_input::is_tool_result_only_user_record;
use super::{ParsedTranscript, ParserAdditions};

mod tail_window;

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
        let pending_uuid = pending_record
            .as_ref()
            .and_then(|record| record.get("uuid").and_then(Value::as_str))
            .map(str::to_owned);
        let pending_node_inserted = if let (Some(record), Some(uuid)) =
            (pending_record.as_ref(), pending_uuid.as_deref())
        {
            if transcript_nodes.contains_key(uuid) {
                false
            } else {
                transcript_nodes.insert(
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
        // The unfinished row selects the render branch.
        // Remove it before caching so append can ingest the completed line.
        mark_active_branch(
            &mut transcript_nodes,
            pending_uuid.as_deref().or(graph_latest_uuid.as_deref()),
        );
        if pending_node_inserted && let Some(uuid) = pending_uuid.as_deref() {
            transcript_nodes.remove(uuid);
        }

        reader.seek(SeekFrom::Start(0))?;
        let mut state = ClaudeParserState::new();
        state.transcript_nodes = transcript_nodes;
        state.latest_uuid = if pending_uuid.is_some() {
            pending_record.as_ref().and_then(record_parent_uuid)
        } else {
            latest_complete_uuid
        };
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

    pub(in crate::commands::agent::watch_web::messages) fn new_tail_window(
        mut reader: impl BufRead + Seek,
        image_directory: Option<&Path>,
        message_limit: usize,
        pending_task_ids: &[String],
    ) -> io::Result<Option<Self>> {
        const INITIAL_WINDOW_BYTES: u64 = 4 * 1024 * 1024;
        const MAX_WINDOW_BYTES: u64 = 64 * 1024 * 1024;
        const MAX_PREFIX_SEARCH_BYTES: u64 = 256 * 1024 * 1024;
        const MAX_ANCESTRY_SEARCHES: usize = 4096;

        let file_length = reader.seek(SeekFrom::End(0))?;
        let mut window_length = INITIAL_WINDOW_BYTES.min(file_length);
        let window_start = loop {
            let start = file_length.saturating_sub(window_length);
            reader.seek(SeekFrom::Start(start))?;
            let window_size = usize::try_from(window_length)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
            let mut bytes = vec![0; window_size];
            reader.read_exact(&mut bytes)?;
            let aligned_start = if start == 0 {
                0
            } else {
                bytes
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(bytes.len(), |index| index + 1)
            };
            let mut row_offset = 0;
            let rows = bytes[aligned_start..]
                .split(|byte| *byte == b'\n')
                .filter_map(|line| {
                    let offset = row_offset;
                    row_offset += line.len() + 1;
                    let record = serde_json::from_slice::<tail_window::GraphRecord>(line).ok()?;
                    let is_human_input = record.record_type.as_deref() == Some("user")
                        && record.origin_kind() != Some("task-notification")
                        && !serde_json::from_slice::<Value>(line)
                            .ok()
                            .is_some_and(|record| is_tool_result_only_user_record(&record));
                    Some((offset, record, is_human_input))
                })
                .collect::<Vec<_>>();
            let user_inputs = rows.iter().filter(|(_, _, is_user)| *is_user).count();
            if message_limit > 0 && user_inputs >= message_limit {
                let selected_index = rows
                    .iter()
                    .enumerate()
                    .rev()
                    .filter(|(_, (_, _, is_user))| *is_user)
                    .nth(message_limit - 1)
                    .map(|(index, _)| index)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "tail window start was lost")
                    })?;
                let selected_offset = rows[selected_index].0;
                break start + aligned_start as u64 + selected_offset as u64;
            }
            if start == 0 {
                break 0;
            }
            if window_length == MAX_WINDOW_BYTES {
                return Ok(None);
            }
            window_length = (window_length.saturating_mul(2)).min(MAX_WINDOW_BYTES);
        };
        if window_start == 0 {
            return Ok(None);
        }

        let mut window_bytes = vec![
            0;
            usize::try_from(file_length - window_start).map_err(
                |error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()),
            )?
        ];
        reader.seek(SeekFrom::Start(window_start))?;
        reader.read_exact(&mut window_bytes)?;
        let selected_records = window_bytes
            .split(|byte| *byte == b'\n')
            .filter_map(parse_record)
            .collect::<Vec<_>>();

        let mut graph_records = selected_records
            .iter()
            .filter_map(|record| {
                let uuid = record.get("uuid").and_then(Value::as_str)?;
                Some((uuid.to_owned(), record_parent_uuid(record)))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut unresolved = graph_records
            .values()
            .filter_map(Clone::clone)
            .filter(|uuid| !graph_records.contains_key(uuid))
            .collect::<std::collections::BTreeSet<_>>();
        let missing_task_ids = pending_task_ids
            .iter()
            .filter(|task_id| {
                !selected_records
                    .iter()
                    .any(|record| tail_window::record_has_task_id(record, task_id))
            })
            .collect::<Vec<_>>();
        let needs_skill_baseline = !selected_records
            .iter()
            .any(tail_window::resets_skill_listing_state);
        if window_start > MAX_PREFIX_SEARCH_BYTES
            && (!unresolved.is_empty()
                || !missing_task_ids.is_empty()
                || (needs_skill_baseline && window_start > 0))
        {
            return Ok(None);
        }
        if missing_task_ids.len() > MAX_ANCESTRY_SEARCHES {
            return Ok(None);
        }
        let mut searches = 0;
        if !unresolved.is_empty()
            && !tail_window::resolve_ancestry_from_prefix(
                &mut reader,
                window_start,
                &mut unresolved,
                &mut graph_records,
                MAX_ANCESTRY_SEARCHES,
                &mut searches,
            )?
        {
            return Ok(None);
        }

        let mut helper_records = Vec::new();
        if needs_skill_baseline {
            let (skill_listings, complete) =
                tail_window::find_skill_listing_prefix_state(&mut reader, window_start)?;
            if !complete {
                return Ok(None);
            }
            helper_records.extend(
                skill_listings
                    .into_iter()
                    .map(|record| (record_timestamp(&record), record)),
            );
        }
        for task_id in missing_task_ids {
            let (result, complete) =
                tail_window::find_background_task_start(&mut reader, window_start, task_id)?;
            if !complete {
                return Ok(None);
            }
            if let Some(result) = result {
                if let Some(uuid) = result.get("uuid").and_then(Value::as_str) {
                    graph_records.insert(uuid.to_owned(), record_parent_uuid(&result));
                }
                if let Some(parent_uuid) = record_parent_uuid(&result) {
                    let (tool_use, complete) =
                        tail_window::find_record_by_uuid(&mut reader, window_start, &parent_uuid)?;
                    if !complete {
                        return Ok(None);
                    }
                    if let Some(tool_use) = tool_use {
                        if let Some(uuid) = tool_use.get("uuid").and_then(Value::as_str) {
                            graph_records.insert(uuid.to_owned(), record_parent_uuid(&tool_use));
                        }
                        helper_records.push((record_timestamp(&tool_use), tool_use));
                    }
                }
                helper_records.push((record_timestamp(&result), result));
            }
        }
        helper_records.sort_by_key(|(timestamp, _)| *timestamp);

        let mut selected_bytes = Vec::new();
        for (uuid, parent_uuid) in graph_records.iter().filter(|(uuid, _)| {
            !selected_records
                .iter()
                .any(|record| record.get("uuid").and_then(Value::as_str) == Some(uuid.as_str()))
        }) {
            let mut record = serde_json::Map::new();
            record.insert("uuid".to_owned(), Value::String(uuid.clone()));
            if let Some(parent_uuid) = parent_uuid {
                record.insert("parentUuid".to_owned(), Value::String(parent_uuid.clone()));
            } else {
                record.insert("parentUuid".to_owned(), Value::Null);
            }
            let projected = serde_json::to_string(&record)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
            selected_bytes.extend_from_slice(projected.as_bytes());
            selected_bytes.push(b'\n');
        }
        for (_, record) in helper_records {
            let encoded = serde_json::to_string(&record)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
            selected_bytes.extend_from_slice(encoded.as_bytes());
            selected_bytes.push(b'\n');
        }
        selected_bytes.extend_from_slice(&window_bytes);

        let tail_window_start = selected_records
            .iter()
            .find(|record| tail_window::is_human_input_record(record))
            .and_then(super::super::record_timestamp);
        let mut parser = Self::new(std::io::Cursor::new(selected_bytes), image_directory)?;
        parser.tail_window_start = tail_window_start;
        parser.tail_window_start_offset = Some(window_start);
        if let Some(cutoff) = tail_window_start {
            parser
                .state
                .events
                .retain(|event| event.timestamp.is_none_or(|timestamp| timestamp >= cutoff));
        }
        Ok(Some(parser))
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
            && is_tool_result_only_user_record(&record)
            && parent_uuid.as_deref() == state.tool_result_parent.as_deref()
            && parent_uuid.as_deref() != state.latest_uuid.as_deref();
        state.transcript_nodes.insert(
            uuid.to_owned(),
            TranscriptNode {
                parent_uuid,
                membership: BranchMembership::Unknown,
            },
        );
        mark_active_branch(&mut state.transcript_nodes, Some(uuid));
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
        .is_some_and(|node| node.membership == BranchMembership::Active);
    if !parent_is_active {
        return false;
    }

    let parent_is_latest = state.latest_uuid.as_deref() == Some(parent_uuid.as_str());
    if is_tool_result_only {
        return parent_is_latest
            || state.tool_result_parent.as_deref() == Some(parent_uuid.as_str());
    }
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
