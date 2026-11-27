use super::super::super::transcript_branch::{
    BranchMembership, is_inactive_branch, mark_active_branch,
};
use super::super::ClaudeParserState;

impl ClaudeParserState {
    pub(in crate::commands::agent::watch_web::messages::parser::claude) fn requires_reparse_for_processed_branch_records(
        &self,
        new_tip_parent_uuid: &str,
    ) -> bool {
        let mut future_nodes = self.transcript_nodes.clone();
        for node in future_nodes.values_mut() {
            node.membership = BranchMembership::Unknown;
        }
        mark_active_branch(&mut future_nodes, Some(new_tip_parent_uuid));

        let uuids = future_nodes.keys().cloned().collect::<Vec<_>>();
        for uuid in uuids {
            if future_nodes
                .get(&uuid)
                .is_some_and(|node| node.membership == BranchMembership::Unknown)
            {
                is_inactive_branch(&uuid, &mut future_nodes);
            }
        }

        self.transcript_nodes.iter().any(|(uuid, node)| {
            if node.is_inert_attachment {
                return false;
            }
            let membership = future_nodes
                .get(uuid)
                .map(|node| node.membership)
                .unwrap_or(BranchMembership::Disconnected);
            if membership == BranchMembership::Active {
                return !node.processed;
            }
            if membership != BranchMembership::ReachesActive {
                return node.processed;
            }
            if node.is_queued_peer_delivery {
                return true;
            }
            let parent_membership = node
                .parent_uuid
                .as_deref()
                .and_then(|parent_uuid| future_nodes.get(parent_uuid))
                .map(|node| node.membership);
            let parent = node
                .parent_uuid
                .as_deref()
                .and_then(|parent_uuid| self.transcript_nodes.get(parent_uuid));
            let current_renderability = match (
                node.is_assistant,
                node.is_tool_result_only,
                node.is_task_notification,
                parent_membership,
            ) {
                (_, _, true, _) => false,
                (true, _, _, Some(BranchMembership::Active)) => {
                    node.has_tool_use
                        && node.parent_uuid.as_deref().is_some_and(|parent_uuid| {
                            self.branch_tracking
                                .is_side_branch_tool_use_assistant_uuid(parent_uuid)
                        })
                }
                (true, _, _, Some(BranchMembership::ReachesActive)) => {
                    node.parent_uuid.as_deref().is_some_and(|parent_uuid| {
                        self.branch_tracking.is_valid_tool_result_uuid(parent_uuid)
                    })
                }
                (false, true, _, Some(BranchMembership::Active)) => {
                    node.was_valid_tool_result
                        || self.branch_tracking.is_valid_tool_result_uuid(uuid)
                }
                (false, true, _, Some(BranchMembership::ReachesActive)) => {
                    (node.was_valid_tool_result
                        || self.branch_tracking.is_valid_tool_result_uuid(uuid))
                        && node.parent_uuid.as_deref().is_some_and(|parent_uuid| {
                            self.branch_tracking.is_tool_use_assistant_uuid(parent_uuid)
                        })
                }
                _ => false,
            };
            let possible_full_renderability = match (
                node.is_assistant,
                node.is_tool_result_only,
                node.is_task_notification,
                parent_membership,
            ) {
                (_, _, true, _) => false,
                (true, _, _, Some(BranchMembership::Active)) => {
                    node.has_tool_use && parent.is_some_and(|parent| parent.has_tool_use)
                }
                (true, _, _, Some(BranchMembership::ReachesActive)) => node
                    .parent_uuid
                    .as_deref()
                    .and_then(|parent_uuid| self.transcript_nodes.get(parent_uuid))
                    .is_some_and(|parent| {
                        parent.is_tool_result_only
                            && (parent.was_valid_tool_result
                                || self.branch_tracking.is_valid_tool_result_uuid(
                                    node.parent_uuid.as_deref().unwrap_or_default(),
                                ))
                    }),
                (false, true, _, Some(BranchMembership::Active)) => parent.is_some_and(|parent| {
                    parent.has_tool_use
                        || parent.was_valid_tool_result
                        || node.was_valid_tool_result
                }),
                (false, true, _, Some(BranchMembership::ReachesActive)) => {
                    parent.is_some_and(|parent| parent.has_tool_use)
                }
                _ => node.is_user,
            };
            if node.processed {
                !current_renderability
            } else {
                possible_full_renderability
            }
        })
    }
}
