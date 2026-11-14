use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read, Seek, SeekFrom};

use memchr::{memchr, memmem};
use serde_json::Value;

use super::super::super::queue::{Queue, QueueReplayEvent};
use super::super::transcript_branch::{BranchMembership, TranscriptNode};
use super::super::user_input::is_tool_result_only_user_record;
use super::parse_record;
use crate::commands::agent::store::WatchWebQueuedPrompt;

pub(super) fn is_human_input_record(record: &Value) -> bool {
    is_human_input(
        record.get("type").and_then(Value::as_str),
        record
            .get("origin")
            .and_then(|origin| origin.get("kind"))
            .and_then(Value::as_str),
        is_tool_result_only_user_record(record),
    )
}

pub(super) fn is_human_input_line(record: &GraphRecord, line: &[u8]) -> bool {
    let is_tool_result_only = record.record_type.as_deref() == Some("user")
        && parse_record(line).is_some_and(|value| is_tool_result_only_user_record(&value));
    is_human_input(
        record.record_type.as_deref(),
        record.origin_kind(),
        is_tool_result_only,
    )
}

fn is_human_input(
    record_type: Option<&str>,
    origin_kind: Option<&str>,
    is_tool_result_only: bool,
) -> bool {
    record_type == Some("user") && !is_tool_result_only && origin_kind != Some("task-notification")
}

pub(super) fn is_skill_listing_record(record: &Value) -> bool {
    record
        .get("attachment")
        .and_then(|attachment| attachment.get("type"))
        .and_then(Value::as_str)
        == Some("skill_listing")
}

pub(super) fn resets_skill_listing_state(record: &Value) -> bool {
    is_skill_listing_record(record)
        && record
            .get("attachment")
            .and_then(|attachment| attachment.get("isInitial"))
            .and_then(Value::as_bool)
            != Some(false)
}

pub(super) fn find_records_by_uuid<R: Read + Seek>(
    reader: &mut R,
    prefix_length: u64,
    uuids: &HashSet<String>,
) -> io::Result<(BTreeMap<String, Value>, bool)> {
    let mut unresolved = uuids.clone();
    let mut matching_records = BTreeMap::new();
    if unresolved.is_empty() {
        return Ok((matching_records, true));
    }
    let complete_scan = visit_prefix_lines_reverse(reader, prefix_length, |line| {
        if memmem::find(line, b"\"uuid\"").is_none() {
            return true;
        }
        let Ok(graph_record) = serde_json::from_slice::<GraphRecord>(line) else {
            return true;
        };
        let Some(uuid) = graph_record.uuid.as_deref() else {
            return true;
        };
        if !unresolved.remove(uuid) {
            return true;
        }
        if let Some(record) = parse_record(line) {
            matching_records.insert(uuid.to_owned(), record);
        }
        !unresolved.is_empty()
    })?;
    let complete = complete_scan || unresolved.is_empty();
    Ok((matching_records, complete))
}

#[derive(serde::Deserialize)]
pub(super) struct GraphRecord {
    pub(super) uuid: Option<String>,
    #[serde(rename = "type")]
    pub(super) record_type: Option<String>,
    #[serde(rename = "origin")]
    origin: Option<GraphOrigin>,
    #[serde(rename = "parentUuid")]
    parent_uuid: Option<Value>,
    #[serde(rename = "logicalParentUuid")]
    logical_parent_uuid: Option<Value>,
}

#[derive(serde::Deserialize)]
struct GraphOrigin {
    kind: Option<String>,
}

impl GraphRecord {
    pub(super) fn origin_kind(&self) -> Option<&str> {
        self.origin
            .as_ref()
            .and_then(|origin| origin.kind.as_deref())
    }

    fn parent_uuid(&self) -> Option<String> {
        match &self.parent_uuid {
            Some(Value::String(parent)) => Some(parent.clone()),
            Some(Value::Null) => self
                .logical_parent_uuid
                .as_ref()
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        }
    }
}

pub(super) fn resolve_ancestry_from_prefix(
    reader: &mut (impl Read + Seek),
    prefix_length: u64,
    unresolved: &mut std::collections::BTreeSet<String>,
    graph_records: &mut std::collections::BTreeMap<String, Option<String>>,
    max_searches: usize,
    searches: &mut usize,
) -> io::Result<bool> {
    let mut exceeded_search_limit = false;
    let complete_scan = visit_prefix_lines_reverse(reader, prefix_length, |line| {
        if unresolved.is_empty() {
            return false;
        }
        if memmem::find(line, b"\"uuid\"").is_none() {
            return true;
        }
        let Ok(record) = serde_json::from_slice::<GraphRecord>(line) else {
            return true;
        };
        let Some(uuid) = record.uuid.as_deref() else {
            return true;
        };
        if !unresolved.remove(uuid) {
            return true;
        }
        *searches += 1;
        if *searches > max_searches {
            exceeded_search_limit = true;
            return false;
        }
        let parent_uuid = record.parent_uuid();
        if let Some(parent_uuid) = &parent_uuid
            && !graph_records.contains_key(parent_uuid)
        {
            unresolved.insert(parent_uuid.clone());
        }
        graph_records.insert(uuid.to_owned(), parent_uuid);
        !unresolved.is_empty()
    })?;
    Ok(!exceeded_search_limit && (complete_scan || unresolved.is_empty()))
}

pub(super) fn find_background_task_starts<R: Read + Seek>(
    reader: &mut R,
    prefix_length: u64,
    task_ids: &[String],
) -> io::Result<(BTreeMap<String, Value>, bool)> {
    let mut unresolved = task_ids.iter().cloned().collect::<HashSet<_>>();
    let mut matching_records = BTreeMap::new();
    if unresolved.is_empty() {
        return Ok((matching_records, true));
    }
    let complete_scan = visit_prefix_lines_reverse(reader, prefix_length, |line| {
        if memmem::find(line, b"toolUseResult").is_none() {
            return true;
        }
        let Some(record) = parse_record(line) else {
            return true;
        };
        if record.get("type").and_then(Value::as_str) != Some("user") {
            return true;
        }
        let Some(result) = record.get("toolUseResult").and_then(Value::as_object) else {
            return true;
        };
        for key in [
            "task_id",
            "taskId",
            "background_task_id",
            "backgroundTaskId",
            "shell_id",
            "shellId",
            "agentId",
        ] {
            let Some(task_id) = result.get(key).and_then(Value::as_str) else {
                continue;
            };
            if unresolved.remove(task_id) {
                matching_records.insert(task_id.to_owned(), record.clone());
            }
        }
        !unresolved.is_empty()
    })?;
    let complete = complete_scan || unresolved.is_empty();
    Ok((matching_records, complete))
}

pub(super) fn find_queued_prompt_deliveries<R: Read + Seek>(
    reader: &mut R,
    prefix_length: u64,
    queued_prompts: &[WatchWebQueuedPrompt],
    user_marker: Option<&str>,
    transcript_nodes: &std::collections::BTreeMap<String, TranscriptNode>,
) -> io::Result<(Vec<i64>, bool)> {
    const MAX_MARKER_RECORDS: usize = 4096;

    if !queued_prompts
        .iter()
        .any(|prompt| prompt.message_id.is_some())
    {
        return Ok((Vec::new(), true));
    }

    let mut queue = Queue::new(queued_prompts);
    let mut marker_records = 0;
    let mut exceeded_record_limit = false;
    let complete_scan = visit_prefix_lines_reverse(reader, prefix_length, |line| {
        if memmem::find(line, b"armyknife-queued-prompt-id:").is_none() {
            return true;
        }
        marker_records += 1;
        if marker_records > MAX_MARKER_RECORDS {
            exceeded_record_limit = true;
            return false;
        }
        let Some(record) = parse_record(line) else {
            return true;
        };
        let Some(event) = QueueReplayEvent::from_record(&record) else {
            return true;
        };
        let on_active_branch = event.delivery_branch_uuid.as_deref().is_none_or(|uuid| {
            transcript_nodes
                .get(uuid)
                .is_some_and(|node| node.membership == BranchMembership::Active)
        });
        if on_active_branch {
            queue.observe_queued_prompt_delivery(&event, user_marker);
            if queue.all_queued_prompt_ids_delivered() {
                return false;
            }
        }
        true
    })?;
    Ok((
        queue.resolved_queued_prompt_ids(),
        !exceeded_record_limit && (complete_scan || queue.all_queued_prompt_ids_delivered()),
    ))
}

pub(super) fn find_skill_listing_prefix_state<R: Read + Seek>(
    reader: &mut R,
    prefix_length: u64,
) -> io::Result<(Vec<Value>, bool)> {
    const MAX_SKILL_LISTINGS: usize = 4096;

    let mut listings = Vec::new();
    let mut found_reset = false;
    let mut exceeded_search_limit = false;
    let complete_scan = visit_prefix_lines_reverse(reader, prefix_length, |line| {
        if memmem::find(line, b"skill_listing").is_none() {
            return true;
        }
        if let Some(record) = parse_record(line)
            && is_skill_listing_record(&record)
        {
            let resets_state = resets_skill_listing_state(&record);
            listings.push(record);
            if resets_state {
                found_reset = true;
                return false;
            }
            if listings.len() >= MAX_SKILL_LISTINGS {
                exceeded_search_limit = true;
                return false;
            }
        }
        true
    })?;
    listings.reverse();
    let complete = found_reset || (complete_scan && !exceeded_search_limit);
    Ok((listings, complete))
}

pub(super) fn record_has_task_id(record: &Value, task_id: &str) -> bool {
    record
        .get("toolUseResult")
        .and_then(Value::as_object)
        .is_some_and(|result| {
            [
                "task_id",
                "taskId",
                "background_task_id",
                "backgroundTaskId",
                "shell_id",
                "shellId",
                "agentId",
            ]
            .into_iter()
            .any(|key| result.get(key).and_then(Value::as_str) == Some(task_id))
        })
}

fn visit_prefix_lines_reverse<R: Read + Seek>(
    reader: &mut R,
    prefix_length: u64,
    mut visit: impl FnMut(&[u8]) -> bool,
) -> io::Result<bool> {
    const CHUNK_BYTES: u64 = 64 * 1024;
    const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

    let mut end = prefix_length;
    let mut carry = Vec::new();
    while end > 0 {
        let start = end.saturating_sub(CHUNK_BYTES);
        let chunk_length = usize::try_from(end - start)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        let mut block = vec![0; chunk_length];
        reader.seek(SeekFrom::Start(start))?;
        reader.read_exact(&mut block)?;
        block.extend_from_slice(&carry);

        if start == 0 {
            for line in block.rsplit(|byte| *byte == b'\n') {
                if line.is_empty() && block.last() == Some(&b'\n') {
                    continue;
                }
                if !visit(line) {
                    return Ok(false);
                }
            }
            return Ok(true);
        }

        let Some(first_newline) = memchr(b'\n', &block) else {
            if block.len() > MAX_LINE_BYTES {
                return Ok(false);
            }
            carry = block;
            end = start;
            continue;
        };
        for line in block[first_newline + 1..].rsplit(|byte| *byte == b'\n') {
            if line.is_empty() && block.last() == Some(&b'\n') {
                continue;
            }
            if !visit(line) {
                return Ok(false);
            }
        }
        carry = block[..=first_newline].to_vec();
        if carry.len() > MAX_LINE_BYTES {
            return Ok(false);
        }
        end = start;
    }
    Ok(true)
}
