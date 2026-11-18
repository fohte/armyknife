use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read, Seek, SeekFrom};

use memchr::{memchr, memmem};
use serde_json::Value;

use super::super::super::queue::{Queue, QueueReplayEvent};
use super::super::super::record_timestamp;
use super::super::state::record_parent_uuid;
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

#[derive(serde::Deserialize)]
pub(super) struct GraphRecord {
    #[serde(rename = "type")]
    pub(super) record_type: Option<String>,
    #[serde(rename = "origin")]
    origin: Option<GraphOrigin>,
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
}

pub(super) fn find_prefix_helpers<R: Read + Seek>(
    reader: &mut R,
    prefix_length: u64,
    task_ids: &[String],
    needs_skill_baseline: bool,
    selected_uuids: &HashSet<String>,
    active_ancestor_uuid: Option<String>,
) -> io::Result<(Vec<Value>, Vec<String>, bool)> {
    const MAX_SKILL_LISTINGS: usize = 4096;

    if task_ids.is_empty() && !needs_skill_baseline && active_ancestor_uuid.is_none() {
        return Ok((Vec::new(), Vec::new(), true));
    }

    let mut unresolved = task_ids.iter().cloned().collect::<HashSet<_>>();
    let mut task_records = BTreeMap::new();
    let mut parent_uuids = HashSet::<String>::new();
    let mut parent_records = BTreeMap::new();
    let mut unresolved_active_uuids = active_ancestor_uuid.into_iter().collect::<HashSet<_>>();
    let mut active_ancestor_uuids = Vec::new();
    let mut resolved_active_uuids = HashSet::new();
    let mut skill_listings = Vec::new();
    let mut found_skill_reset = !needs_skill_baseline;
    let mut exceeded_skill_limit = false;
    let complete_scan = visit_prefix_lines_reverse(reader, prefix_length, |line| {
        let has_requested_task_id = unresolved
            .iter()
            .any(|task_id| memmem::find(line, task_id.as_bytes()).is_some());
        let has_task_notification =
            has_requested_task_id && memmem::find(line, b"<task-notification>").is_some();
        let has_task_result =
            has_requested_task_id && memmem::find(line, b"toolUseResult").is_some();
        let has_task_event = has_task_notification || has_task_result;
        let has_uuid = parent_uuids
            .iter()
            .chain(&unresolved_active_uuids)
            .any(|uuid| memmem::find(line, uuid.as_bytes()).is_some());
        let has_skill_listing =
            !found_skill_reset && memmem::find(line, b"skill_listing").is_some();
        if !has_task_event && !has_uuid && !has_skill_listing {
            return true;
        }
        let Some(record) = parse_record(line) else {
            return true;
        };
        if has_task_event {
            if has_task_notification
                && let Some(text) = super::super::background_tasks::task_notification_text(&record)
                && let Some(task_id) =
                    super::super::background_tasks::finished_task_notification_id(&text)
            {
                unresolved.remove(task_id);
            }
            if record.get("type").and_then(Value::as_str) == Some("user") {
                for key in [
                    "task_id",
                    "taskId",
                    "background_task_id",
                    "backgroundTaskId",
                    "shell_id",
                    "shellId",
                    "agentId",
                ] {
                    let Some(task_id) = record
                        .get("toolUseResult")
                        .and_then(|result| result.get(key))
                        .and_then(Value::as_str)
                    else {
                        continue;
                    };
                    if unresolved.remove(task_id) {
                        if let Some(parent_uuid) = record_parent_uuid(&record) {
                            parent_uuids.insert(parent_uuid);
                        }
                        task_records.insert(task_id.to_owned(), record.clone());
                    }
                }
            }
        }
        if has_uuid && let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
            if parent_uuids.remove(uuid) {
                parent_records.insert(uuid.to_owned(), record.clone());
            }
            if unresolved_active_uuids.remove(uuid) {
                active_ancestor_uuids.push(uuid.to_owned());
                resolved_active_uuids.insert(uuid.to_owned());
                if let Some(parent_uuid) = record_parent_uuid(&record)
                    && !selected_uuids.contains(&parent_uuid)
                    && !resolved_active_uuids.contains(&parent_uuid)
                {
                    unresolved_active_uuids.insert(parent_uuid);
                }
            }
        }
        if has_skill_listing && is_skill_listing_record(&record) {
            let resets_state = resets_skill_listing_state(&record);
            skill_listings.push(record);
            if resets_state {
                found_skill_reset = true;
            } else if skill_listings.len() >= MAX_SKILL_LISTINGS {
                exceeded_skill_limit = true;
            }
        }
        if exceeded_skill_limit {
            return false;
        }
        let found_task_records = unresolved.is_empty();
        let found_parents = parent_uuids.is_empty();
        if found_task_records
            && found_parents
            && found_skill_reset
            && unresolved_active_uuids.is_empty()
        {
            return false;
        }
        true
    })?;

    skill_listings.reverse();
    let all_requested_records_found = unresolved.is_empty() && parent_uuids.is_empty();
    let complete = !exceeded_skill_limit
        && (found_skill_reset || complete_scan)
        && (complete_scan || all_requested_records_found)
        && (complete_scan || unresolved_active_uuids.is_empty());
    let mut records = skill_listings;
    records.extend(parent_records.into_values());
    records.extend(task_records.into_values());
    records.sort_by_key(record_timestamp);
    Ok((records, active_ancestor_uuids, complete))
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
    const CHUNK_BYTES: u64 = 16 * 1024 * 1024;
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
