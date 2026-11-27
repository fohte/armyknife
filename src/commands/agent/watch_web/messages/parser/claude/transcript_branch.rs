use std::collections::HashMap;

use serde_json::Value;

use super::background_tasks::task_notification_text;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BranchMembership {
    Unknown,
    Visiting,
    Active,
    ReachesActive,
    Disconnected,
}

#[derive(Clone)]
pub(super) struct TranscriptNode {
    pub(super) uuid_id: usize,
    pub(super) record_order: u64,
    pub(super) parent_uuid: Option<String>,
    pub(super) membership: BranchMembership,
    pub(super) has_tool_use: bool,
    pub(super) tool_use_ids: Option<Box<[String]>>,
    pub(super) tool_result_ids: Option<Box<[String]>>,
    pub(super) is_tool_result_only: bool,
    pub(super) is_task_notification: bool,
}

#[derive(Clone, Default)]
pub(super) struct TranscriptContentMetadata {
    pub(super) has_tool_use: bool,
    pub(super) tool_use_ids: Vec<String>,
    pub(super) tool_result_ids: Vec<String>,
    pub(super) is_tool_result_only: bool,
    pub(super) tool_use_count: usize,
}

impl TranscriptContentMetadata {
    pub(super) fn from_record(record: &Value) -> Self {
        let record_type = record.get("type").and_then(Value::as_str);
        let mut metadata = Self::default();
        if matches!(record_type, Some("assistant" | "user"))
            && let Some(blocks) = record
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(Value::as_array)
        {
            let mut has_user_content = false;
            let mut has_non_result_user_content = false;
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("tool_use") if record_type == Some("assistant") => {
                        metadata.has_tool_use = true;
                        metadata.tool_use_count += 1;
                        if let Some(id) = block
                            .get("id")
                            .or_else(|| block.get("tool_use_id"))
                            .and_then(Value::as_str)
                        {
                            metadata.tool_use_ids.push(id.to_owned());
                        }
                    }
                    Some("tool_result") if record_type == Some("user") => {
                        has_user_content = true;
                        if let Some(id) = block
                            .get("id")
                            .or_else(|| block.get("tool_use_id"))
                            .and_then(Value::as_str)
                        {
                            metadata.tool_result_ids.push(id.to_owned());
                        }
                    }
                    _ if record_type == Some("user") => {
                        has_user_content = true;
                        has_non_result_user_content = true;
                    }
                    _ => {}
                }
            }
            metadata.is_tool_result_only =
                record_type == Some("user") && has_user_content && !has_non_result_user_content;
        }
        metadata
    }
}

impl TranscriptNode {
    pub(super) fn from_simple_append(
        parent_uuid: Option<String>,
        uuid_id: usize,
        record_order: u64,
        tool_use_flags: (bool, bool, bool),
    ) -> Self {
        let (has_tool_use, is_tool_result_only, is_task_notification) = tool_use_flags;
        Self {
            uuid_id,
            record_order,
            parent_uuid,
            membership: BranchMembership::Unknown,
            has_tool_use,
            tool_use_ids: None,
            tool_result_ids: None,
            is_tool_result_only,
            is_task_notification,
        }
    }

    pub(super) fn from_append_metadata(
        parent_uuid: Option<String>,
        uuid_id: usize,
        record_order: u64,
        metadata: TranscriptContentMetadata,
        is_task_notification: bool,
    ) -> Self {
        Self {
            uuid_id,
            record_order,
            parent_uuid,
            membership: BranchMembership::Unknown,
            has_tool_use: metadata.has_tool_use,
            tool_use_ids: (!metadata.tool_use_ids.is_empty())
                .then(|| metadata.tool_use_ids.into_boxed_slice()),
            tool_result_ids: (!metadata.tool_result_ids.is_empty())
                .then(|| metadata.tool_result_ids.into_boxed_slice()),
            is_tool_result_only: metadata.is_tool_result_only,
            is_task_notification,
        }
    }

    pub(super) fn from_record(
        record: &Value,
        parent_uuid: Option<String>,
        uuid_id: usize,
        record_order: u64,
    ) -> Self {
        let metadata = TranscriptContentMetadata::from_record(record);
        let is_task_notification =
            !metadata.is_tool_result_only && task_notification_text(record).is_some();
        Self {
            uuid_id,
            record_order,
            parent_uuid,
            membership: BranchMembership::Unknown,
            has_tool_use: metadata.has_tool_use,
            tool_use_ids: (!metadata.tool_use_ids.is_empty())
                .then(|| metadata.tool_use_ids.into_boxed_slice()),
            tool_result_ids: (!metadata.tool_result_ids.is_empty())
                .then(|| metadata.tool_result_ids.into_boxed_slice()),
            is_tool_result_only: metadata.is_tool_result_only,
            is_task_notification,
        }
    }
}

pub(super) fn content_ids(record: &Value, kind: &str, id_keys: &[&str]) -> Vec<String> {
    record
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some(kind))
        .filter_map(|block| {
            id_keys
                .iter()
                .fold(None, |value, key| value.or_else(|| block.get(*key)))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

pub(super) fn mark_active_branch(
    transcript_nodes: &mut HashMap<String, TranscriptNode>,
    latest_uuid: Option<&str>,
) {
    let Some(mut uuid) = latest_uuid.map(str::to_owned) else {
        return;
    };

    loop {
        let Some(node) = transcript_nodes.get_mut(&uuid) else {
            return;
        };
        if node.membership == BranchMembership::Active {
            return;
        }
        node.membership = BranchMembership::Active;
        let Some(parent_uuid) = &node.parent_uuid else {
            return;
        };
        let parent_uuid = parent_uuid.clone();
        if !transcript_nodes.contains_key(&parent_uuid) {
            return;
        }
        uuid = parent_uuid;
    }
}

pub(super) fn is_inactive_branch(
    uuid: &str,
    transcript_nodes: &mut HashMap<String, TranscriptNode>,
) -> bool {
    let mut current_uuid = uuid.to_owned();
    let mut path = Vec::new();
    let reaches_active = loop {
        let Some(node) = transcript_nodes.get_mut(&current_uuid) else {
            break false;
        };
        match node.membership {
            BranchMembership::Active | BranchMembership::ReachesActive => break true,
            BranchMembership::Disconnected | BranchMembership::Visiting => break false,
            BranchMembership::Unknown => node.membership = BranchMembership::Visiting,
        }
        path.push(current_uuid.clone());
        let Some(parent_uuid) = &node.parent_uuid else {
            break false;
        };
        current_uuid = parent_uuid.clone();
    };

    let membership = if reaches_active {
        BranchMembership::ReachesActive
    } else {
        BranchMembership::Disconnected
    };
    for uuid in path {
        if let Some(node) = transcript_nodes.get_mut(&uuid) {
            node.membership = membership;
        }
    }
    reaches_active
}

pub(super) fn active_record_ids(
    transcript_nodes: &HashMap<String, TranscriptNode>,
    latest_uuid: Option<&str>,
) -> std::collections::HashSet<usize> {
    let record_count = transcript_nodes
        .values()
        .map(|node| node.record_order)
        .max()
        .and_then(|order| usize::try_from(order).ok())
        .and_then(|order| order.checked_add(1));
    let Some(record_count) = record_count else {
        return active_record_ids_sorted(transcript_nodes, latest_uuid);
    };
    let mut record_uuids = vec![None; record_count];
    for (uuid, node) in transcript_nodes {
        let Ok(index) = usize::try_from(node.record_order) else {
            return active_record_ids_sorted(transcript_nodes, latest_uuid);
        };
        if record_uuids[index].replace(uuid.as_str()).is_some() {
            return active_record_ids_sorted(transcript_nodes, latest_uuid);
        }
    }

    let mut memberships = vec![BranchMembership::Unknown; record_count];
    mark_ordered_active_branch(transcript_nodes, &mut memberships, latest_uuid);

    let mut active_ids = std::collections::HashSet::with_capacity(transcript_nodes.len());
    for (index, uuid) in record_uuids.iter().enumerate() {
        let Some(uuid) = uuid else {
            continue;
        };
        let Some(node) = transcript_nodes.get(*uuid) else {
            continue;
        };
        let uuid_id = node.uuid_id;
        let bypass_branch_check = node.is_tool_result_only || node.is_task_notification;
        let active_record = match memberships[index] {
            BranchMembership::Unknown if !bypass_branch_check => {
                !is_ordered_inactive_branch(transcript_nodes, &mut memberships, uuid)
            }
            _ => true,
        };
        if active_record {
            active_ids.insert(uuid_id);
        }
    }
    active_ids
}

fn mark_ordered_active_branch(
    transcript_nodes: &HashMap<String, TranscriptNode>,
    memberships: &mut [BranchMembership],
    latest_uuid: Option<&str>,
) {
    let Some(mut uuid) = latest_uuid else {
        return;
    };
    loop {
        let Some(node) = transcript_nodes.get(uuid) else {
            return;
        };
        let Ok(index) = usize::try_from(node.record_order) else {
            return;
        };
        if memberships[index] == BranchMembership::Active {
            return;
        }
        memberships[index] = BranchMembership::Active;
        let Some(parent_uuid) = node.parent_uuid.as_deref() else {
            return;
        };
        uuid = parent_uuid;
    }
}

fn is_ordered_inactive_branch(
    transcript_nodes: &HashMap<String, TranscriptNode>,
    memberships: &mut [BranchMembership],
    start_uuid: &str,
) -> bool {
    let mut current_uuid = start_uuid;
    let mut path = Vec::new();
    let reaches_active = loop {
        let Some(node) = transcript_nodes.get(current_uuid) else {
            break false;
        };
        let Ok(index) = usize::try_from(node.record_order) else {
            break false;
        };
        match memberships[index] {
            BranchMembership::Active | BranchMembership::ReachesActive => break true,
            BranchMembership::Disconnected | BranchMembership::Visiting => break false,
            BranchMembership::Unknown => memberships[index] = BranchMembership::Visiting,
        }
        path.push(index);
        let Some(parent_uuid) = node.parent_uuid.as_deref() else {
            break false;
        };
        current_uuid = parent_uuid;
    };

    let membership = if reaches_active {
        BranchMembership::ReachesActive
    } else {
        BranchMembership::Disconnected
    };
    for index in path {
        memberships[index] = membership;
    }
    reaches_active
}

fn active_record_ids_sorted(
    transcript_nodes: &HashMap<String, TranscriptNode>,
    latest_uuid: Option<&str>,
) -> std::collections::HashSet<usize> {
    let mut transcript_nodes = transcript_nodes.clone();
    for node in transcript_nodes.values_mut() {
        node.membership = BranchMembership::Unknown;
    }
    mark_active_branch(&mut transcript_nodes, latest_uuid);

    let mut records = transcript_nodes
        .iter()
        .map(|(uuid, node)| {
            (
                node.record_order,
                uuid.clone(),
                node.is_tool_result_only || node.is_task_notification,
            )
        })
        .collect::<Vec<_>>();
    records.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));

    let mut active_ids = std::collections::HashSet::new();
    for (_, uuid, bypass_branch_check) in records {
        let membership = transcript_nodes.get(&uuid).map(|node| node.membership);
        let active_record = match membership {
            Some(BranchMembership::Unknown) if !bypass_branch_check => {
                !is_inactive_branch(&uuid, &mut transcript_nodes)
            }
            Some(_) => true,
            None => bypass_branch_check,
        };
        if active_record && let Some(node) = transcript_nodes.get(&uuid) {
            active_ids.insert(node.uuid_id);
        }
    }

    active_ids
}
