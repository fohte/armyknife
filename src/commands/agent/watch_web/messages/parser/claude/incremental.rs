use std::io::{self, BufRead, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::super::super::parser::jsonl::JsonlLineBuffer;
use super::state::{ClaudeParserState, record_parent_uuid};
use super::transcript_branch::{BranchMembership, TranscriptNode, mark_active_branch};
use super::user_input::is_tool_result_only_user_record;
use super::{ParsedTranscript, ParserAdditions};

#[derive(Clone)]
pub(in crate::commands::agent::watch_web::messages) struct IncrementalClaudeParser {
    state: ClaudeParserState,
    lines: JsonlLineBuffer,
    image_directory: Option<PathBuf>,
    pending_is_initial: bool,
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
                latest_complete_uuid = record
                    .get("uuid")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or(latest_complete_uuid.clone());
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
        mark_active_branch(&mut transcript_nodes, graph_latest_uuid.as_deref());

        reader.seek(SeekFrom::Start(0))?;
        let mut state = ClaudeParserState::new();
        state.transcript_nodes = transcript_nodes;
        state.latest_uuid = latest_complete_uuid;
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
        })
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
        if matches!(origin, RecordOrigin::Appended) && !parallel_tool_result {
            state.latest_uuid = Some(uuid.to_owned());
        }
    }
    state.process_record(record, now);
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

#[cfg(test)]
mod tests;
