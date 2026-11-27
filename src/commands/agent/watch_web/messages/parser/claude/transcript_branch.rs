use std::collections::BTreeMap;

use serde_json::Value;

use super::background_tasks::task_notification_text;
use super::user_input::is_tool_result_only_user_record;

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
    pub(super) record_order: u64,
    pub(super) parent_uuid: Option<String>,
    pub(super) membership: BranchMembership,
    pub(super) has_tool_use: bool,
    pub(super) tool_use_ids: Vec<String>,
    pub(super) tool_result_ids: Vec<String>,
    pub(super) is_tool_result_only: bool,
    pub(super) is_task_notification: bool,
    pub(super) was_valid_tool_result: bool,
    pub(super) processed: bool,
}

impl TranscriptNode {
    pub(super) fn from_record(
        record: &Value,
        parent_uuid: Option<String>,
        record_order: u64,
    ) -> Self {
        Self {
            record_order,
            parent_uuid,
            membership: BranchMembership::Unknown,
            has_tool_use: is_tool_use_assistant(record),
            tool_use_ids: content_ids(record, "tool_use", &["id", "tool_use_id"]),
            tool_result_ids: content_ids(record, "tool_result", &["id", "tool_use_id"]),
            is_tool_result_only: is_tool_result_only_user_record(record),
            is_task_notification: task_notification_text(record).is_some(),
            was_valid_tool_result: false,
            processed: false,
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

pub(super) fn is_tool_use_assistant(record: &Value) -> bool {
    record.get("type").and_then(Value::as_str) == Some("assistant")
        && record
            .get("message")
            .and_then(|message| message.get("content"))
            .is_some_and(|content| {
                content.as_array().is_some_and(|blocks| {
                    blocks
                        .iter()
                        .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
                })
            })
}

pub(super) fn mark_active_branch(
    transcript_nodes: &mut BTreeMap<String, TranscriptNode>,
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
    transcript_nodes: &mut BTreeMap<String, TranscriptNode>,
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

pub(super) fn active_record_uuids(
    transcript_nodes: &BTreeMap<String, TranscriptNode>,
    latest_uuid: Option<&str>,
) -> std::collections::HashSet<String> {
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

    let mut active_uuids = std::collections::HashSet::new();
    for (_, uuid, bypass_branch_check) in records {
        let membership = transcript_nodes.get(&uuid).map(|node| node.membership);
        let active_record = match membership {
            Some(BranchMembership::Unknown) if !bypass_branch_check => {
                !is_inactive_branch(&uuid, &mut transcript_nodes)
            }
            Some(_) => true,
            None => bypass_branch_check,
        };
        if active_record {
            active_uuids.insert(uuid);
        }
    }

    active_uuids
}
