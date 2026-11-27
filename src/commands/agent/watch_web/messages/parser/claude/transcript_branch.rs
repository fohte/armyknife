use std::collections::BTreeMap;

use serde_json::Value;

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
    pub(super) parent_uuid: Option<String>,
    pub(super) membership: BranchMembership,
    pub(super) is_assistant: bool,
    pub(super) has_tool_use: bool,
    pub(super) is_user: bool,
    pub(super) is_tool_result_only: bool,
    pub(super) is_task_notification: bool,
    pub(super) is_inert_attachment: bool,
    pub(super) is_queued_peer_delivery: bool,
    pub(super) was_valid_tool_result: bool,
    pub(super) processed: bool,
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
