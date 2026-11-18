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
    if task_ids.is_empty() && !needs_skill_baseline && active_ancestor_uuid.is_none() {
        return Ok((Vec::new(), Vec::new(), true));
    }

    let mut tasks = PendingTaskPrefix::new(task_ids);
    let mut ancestry = ActiveAncestorPrefix::new(selected_uuids, active_ancestor_uuid);
    let mut skills = SkillListingPrefix::new(needs_skill_baseline);
    let complete_scan = visit_prefix_lines_reverse(reader, prefix_length, |line| {
        let needs_task_record = tasks.needs_event_line(line);
        let needs_parent_record = tasks.needs_parent_line(line);
        let needs_ancestor_record = ancestry.needs_line(line);
        let needs_skill_record = skills.needs_line(line);
        if !needs_task_record
            && !needs_parent_record
            && !needs_ancestor_record
            && !needs_skill_record
        {
            return true;
        }
        let Some(record) = parse_record(line) else {
            return true;
        };
        if needs_task_record {
            tasks.visit_event(line, &record);
        }
        if needs_parent_record {
            tasks.visit_parent(&record);
        }
        if needs_ancestor_record {
            ancestry.visit(&record);
        }
        if needs_skill_record {
            skills.visit(&record);
        }
        if skills.exceeded_limit() {
            return false;
        }
        !(tasks.complete() && ancestry.complete() && skills.complete(false))
    })?;

    let complete = !skills.exceeded_limit()
        && (tasks.complete() || complete_scan)
        && (ancestry.complete() || complete_scan)
        && skills.complete(complete_scan);
    let mut records = skills.into_records();
    records.extend(tasks.into_records());
    records.sort_by_key(record_timestamp);
    Ok((records, ancestry.into_uuids(), complete))
}

struct PendingTaskPrefix {
    unresolved: HashSet<String>,
    task_records: BTreeMap<String, Value>,
    parent_uuids: HashSet<String>,
    parent_records: BTreeMap<String, Value>,
}

impl PendingTaskPrefix {
    fn new(task_ids: &[String]) -> Self {
        Self {
            unresolved: task_ids.iter().cloned().collect(),
            task_records: BTreeMap::new(),
            parent_uuids: HashSet::new(),
            parent_records: BTreeMap::new(),
        }
    }

    fn needs_event_line(&self, line: &[u8]) -> bool {
        !self.unresolved.is_empty()
            && (memmem::find(line, b"toolUseResult").is_some()
                || memmem::find(line, b"<task-notification>").is_some())
    }

    fn needs_parent_line(&self, line: &[u8]) -> bool {
        self.parent_uuids
            .iter()
            .any(|uuid| memmem::find(line, uuid.as_bytes()).is_some())
    }

    fn visit_event(&mut self, line: &[u8], record: &Value) {
        if memmem::find(line, b"<task-notification>").is_some()
            && let Some(text) = super::super::background_tasks::task_notification_text(record)
            && let Some(task_id) =
                super::super::background_tasks::finished_task_notification_id(&text)
        {
            self.unresolved.remove(task_id);
        }
        if record.get("type").and_then(Value::as_str) != Some("user") {
            return;
        }
        let matching_task_ids = task_result_ids(record)
            .filter(|task_id| self.unresolved.remove(*task_id))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if matching_task_ids.is_empty() {
            return;
        }
        if let Some(parent_uuid) = record_parent_uuid(record) {
            self.parent_uuids.insert(parent_uuid);
        }
        for task_id in matching_task_ids {
            self.task_records.insert(task_id, record.clone());
        }
    }

    fn visit_parent(&mut self, record: &Value) {
        if let Some(uuid) = record.get("uuid").and_then(Value::as_str)
            && self.parent_uuids.remove(uuid)
        {
            self.parent_records.insert(uuid.to_owned(), record.clone());
        }
    }

    fn complete(&self) -> bool {
        self.unresolved.is_empty() && self.parent_uuids.is_empty()
    }

    fn into_records(self) -> Vec<Value> {
        self.parent_records
            .into_values()
            .chain(self.task_records.into_values())
            .collect()
    }
}

struct ActiveAncestorPrefix<'a> {
    selected_uuids: &'a HashSet<String>,
    unresolved_uuids: HashSet<String>,
    resolved_uuids: HashSet<String>,
    active_uuids: Vec<String>,
}

impl<'a> ActiveAncestorPrefix<'a> {
    fn new(selected_uuids: &'a HashSet<String>, first_uuid: Option<String>) -> Self {
        Self {
            selected_uuids,
            unresolved_uuids: first_uuid.into_iter().collect(),
            resolved_uuids: HashSet::new(),
            active_uuids: Vec::new(),
        }
    }

    fn needs_line(&self, line: &[u8]) -> bool {
        self.unresolved_uuids
            .iter()
            .any(|uuid| memmem::find(line, uuid.as_bytes()).is_some())
    }

    fn visit(&mut self, record: &Value) {
        let Some(uuid) = record.get("uuid").and_then(Value::as_str) else {
            return;
        };
        if !self.unresolved_uuids.remove(uuid) {
            return;
        }
        self.active_uuids.push(uuid.to_owned());
        self.resolved_uuids.insert(uuid.to_owned());
        if let Some(parent_uuid) = record_parent_uuid(record)
            && !self.selected_uuids.contains(&parent_uuid)
            && !self.resolved_uuids.contains(&parent_uuid)
        {
            self.unresolved_uuids.insert(parent_uuid);
        }
    }

    fn complete(&self) -> bool {
        self.unresolved_uuids.is_empty()
    }

    fn into_uuids(self) -> Vec<String> {
        self.active_uuids
    }
}

struct SkillListingPrefix {
    listings: Vec<Value>,
    found_reset: bool,
    exceeded_limit: bool,
}

impl SkillListingPrefix {
    fn new(needs_baseline: bool) -> Self {
        Self {
            listings: Vec::new(),
            found_reset: !needs_baseline,
            exceeded_limit: false,
        }
    }

    fn needs_line(&self, line: &[u8]) -> bool {
        !self.found_reset && memmem::find(line, b"skill_listing").is_some()
    }

    fn visit(&mut self, record: &Value) {
        const MAX_SKILL_LISTINGS: usize = 4096;

        if !is_skill_listing_record(record) {
            return;
        }
        self.found_reset = resets_skill_listing_state(record);
        self.listings.push(record.clone());
        if !self.found_reset && self.listings.len() >= MAX_SKILL_LISTINGS {
            self.exceeded_limit = true;
        }
    }

    fn complete(&self, complete_scan: bool) -> bool {
        self.found_reset || complete_scan
    }

    fn exceeded_limit(&self) -> bool {
        self.exceeded_limit
    }

    fn into_records(mut self) -> Vec<Value> {
        self.listings.reverse();
        self.listings
    }
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
    task_result_ids(record).any(|id| id == task_id)
}

fn task_result_ids(record: &Value) -> impl Iterator<Item = &str> + '_ {
    const TASK_RESULT_ID_KEYS: [&str; 7] = [
        "task_id",
        "taskId",
        "background_task_id",
        "backgroundTaskId",
        "shell_id",
        "shellId",
        "agentId",
    ];

    TASK_RESULT_ID_KEYS.into_iter().filter_map(move |key| {
        record
            .get("toolUseResult")
            .and_then(|result| result.get(key))
            .and_then(Value::as_str)
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
