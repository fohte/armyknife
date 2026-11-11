use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::super::super::ChatItem;
use super::super::ToolSummary;
use super::transcript_branch::{BranchMembership, TranscriptNode};
use super::{
    classifier, is_inactive_branch, tool_result_blocks, tool_result_failed, tool_result_text,
};
use crate::commands::agent::store::AgentDecision;

pub(super) fn insert_time_placed_decisions(
    record_time: Option<DateTime<Utc>>,
    recorded_decision_seqs: &HashSet<i64>,
    unplaced_decisions: &mut BTreeMap<i64, AgentDecision>,
    tools: &mut ToolSummary,
    items: &mut Vec<ChatItem>,
) {
    let Some(record_time) = record_time else {
        return;
    };
    let mut decisions_to_place = unplaced_decisions
        .values()
        .filter(|decision| !recorded_decision_seqs.contains(&decision.seq))
        .filter_map(|decision| {
            DateTime::parse_from_rfc3339(&decision.created_at)
                .ok()
                .map(|created_at| (created_at.with_timezone(&Utc), decision.seq))
        })
        .filter(|(created_at, _)| record_time > *created_at)
        .collect::<Vec<_>>();
    decisions_to_place.sort_unstable();
    if decisions_to_place.is_empty() {
        return;
    }

    tools.flush_into(items);
    for (_, seq) in decisions_to_place {
        if let Some(decision) = unplaced_decisions.remove(&seq) {
            items.push(decision.into());
        }
    }
}

pub(super) fn observe_recorded_decisions(
    record: &Value,
    tool_uses: &mut Vec<(String, Option<String>)>,
    tool_results: &mut Vec<(String, Option<i64>)>,
) {
    match record.get("type").and_then(Value::as_str) {
        Some("assistant") => {
            let Some(blocks) = record
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(Value::as_array)
            else {
                return;
            };
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                    continue;
                }
                let (Some(id), Some(name)) = (
                    block.get("id").and_then(Value::as_str),
                    block.get("name").and_then(Value::as_str),
                ) else {
                    continue;
                };
                if classifier::is_agent_add_tool(name, block.get("input"), "decision") {
                    tool_uses.push((
                        id.to_owned(),
                        record
                            .get("uuid")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    ));
                }
            }
        }
        Some("user") => {
            let Some(message) = record.get("message") else {
                return;
            };
            for block in tool_result_blocks(message) {
                let Some(id) = block.get("tool_use_id").and_then(Value::as_str) else {
                    continue;
                };
                let seq = (!tool_result_failed(record, block))
                    .then(|| {
                        block
                            .get("content")
                            .and_then(tool_result_text)
                            .and_then(|text| classifier::recorded_agent_add_seq(&text, "decision"))
                    })
                    .flatten();
                tool_results.push((id.to_owned(), seq));
            }
        }
        _ => {}
    }
}

pub(super) fn recorded_decision_seqs(
    tool_uses: &[(String, Option<String>)],
    tool_results: &[(String, Option<i64>)],
    transcript_nodes: &BTreeMap<String, TranscriptNode>,
) -> HashSet<i64> {
    let mut transcript_nodes = transcript_nodes.clone();
    let mut branch_membership_by_uuid = HashMap::<String, bool>::new();
    let mut active_tool_uses = tool_uses
        .iter()
        .filter_map(|(id, uuid)| {
            let Some(uuid) = uuid else {
                return Some(id.clone());
            };
            if let Some(is_active) = branch_membership_by_uuid.get(uuid) {
                return (*is_active).then(|| id.clone());
            }
            let membership = transcript_nodes.get(uuid)?.membership;
            let is_active = membership != BranchMembership::Unknown
                || !is_inactive_branch(uuid, &mut transcript_nodes);
            branch_membership_by_uuid.insert(uuid.clone(), is_active);
            if is_active { Some(id.clone()) } else { None }
        })
        .collect::<Vec<_>>();
    tool_results
        .iter()
        .filter_map(|(id, seq)| {
            let index = active_tool_uses
                .iter()
                .position(|tool_use_id| tool_use_id == id)?;
            active_tool_uses.remove(index);
            *seq
        })
        .collect()
}
