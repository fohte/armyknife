use chrono::{DateTime, Utc};
use serde_json::Value;

use super::super::state::{ClaudeParserState, is_attachment_record, record_parent_uuid};
use super::super::transcript_branch::{BranchMembership, TranscriptNode, mark_active_branch};
use super::continuation::{AppendDecision, AppendHistory, ClassifiedAppend, classify_append};
use super::prepare_pending_render_state;

#[derive(Clone, Copy)]
pub(super) enum RecordOrigin {
    Initial,
    Appended,
    Pending { is_initial: bool },
}

pub(super) fn ingest_record(
    state: &mut ClaudeParserState,
    append_history: &mut AppendHistory,
    record: Value,
    now: DateTime<Utc>,
    origin: RecordOrigin,
    active_cache_updates: Option<&mut Vec<usize>>,
) -> bool {
    let mut classified_append = if matches!(
        origin,
        RecordOrigin::Appended | RecordOrigin::Pending { is_initial: false }
    ) {
        let Some(classified) = classify_append(state, append_history, &record) else {
            return false;
        };
        Some(classified)
    } else {
        None
    };
    state.has_compact_boundary_record |= super::is_compact_boundary(&record);
    let append_decision = classified_append
        .as_ref()
        .map(|classified| classified.decision);
    let active_cache_update = classified_append
        .as_ref()
        .filter(|classified| classified.extends_active_record_cache())
        .and_then(|_| {
            record
                .get("uuid")
                .and_then(Value::as_str)
                .map(|_| state.next_uuid_id)
        });
    let is_simple_tail_append = matches!(origin, RecordOrigin::Appended)
        && append_decision == Some(AppendDecision::SimpleTail);
    let simple_node_flags = (append_decision == Some(AppendDecision::SimpleTail)
        && !state.has_transcript_forks
        && !state.has_duplicate_uuids
        && state.forward_parent_uuids.is_empty()
        && state.all_transcript_records_active
        && !state.has_compact_boundary_record)
        .then(|| {
            classified_append
                .as_ref()
                .map(|classified| classified.simple_node_flags())
        })
        .flatten();
    if matches!(origin, RecordOrigin::Pending { .. }) {
        prepare_pending_render_state(state, &record);
    }
    if !matches!(origin, RecordOrigin::Initial)
        && let Some(uuid) = record.get("uuid").and_then(Value::as_str)
    {
        let parent_uuid = record_parent_uuid(&record);
        let record_order = state.transcript_parent_child_counts.len() as u64;
        let uuid_id = state.next_uuid_id;
        if matches!(origin, RecordOrigin::Appended) {
            state.transcript_parent_child_counts.push(0);
            state.next_uuid_id += 1;
        }
        let mut node = if let Some(flags) = simple_node_flags {
            TranscriptNode::from_simple_append(parent_uuid.clone(), uuid_id, record_order, flags)
        } else if let Some(classified) = classified_append.as_mut() {
            classified.transcript_node(uuid_id, record_order)
        } else {
            TranscriptNode::from_record(&record, parent_uuid.clone(), uuid_id, record_order)
        };
        if is_simple_tail_append {
            node.membership = BranchMembership::Active;
        }
        state.transcript_nodes.insert(uuid.to_owned(), node);
        if is_simple_tail_append && !state.has_transcript_forks {
            if let Some(parent_uuid) = parent_uuid.as_deref()
                && let Some(parent) = state.transcript_nodes.get(parent_uuid)
                && let Some(child_count) = state
                    .transcript_parent_child_counts
                    .get_mut(parent.record_order as usize)
            {
                *child_count = 1;
            }
        } else {
            state.index_transcript_child(parent_uuid.as_deref(), uuid);
        }
        if !is_simple_tail_append {
            mark_active_branch(&mut state.transcript_nodes, Some(uuid));
        }
        if matches!(origin, RecordOrigin::Appended) {
            state.latest_record_uuid = Some(uuid.to_owned());
            if !is_attachment_record(&record) {
                state.latest_uuid = Some(uuid.to_owned());
            }
        }
    }
    state.process_record_with_pending_tool_result(
        &record,
        now,
        classified_append
            .as_ref()
            .map(ClassifiedAppend::pending_tool_result),
    );
    match origin {
        RecordOrigin::Initial => append_history.observe_initial(record),
        RecordOrigin::Appended => {
            if let Some(classified) = classified_append {
                append_history.observe_append(classified);
            }
        }
        RecordOrigin::Pending { .. } => {}
    }
    if matches!(origin, RecordOrigin::Appended) {
        match append_decision {
            Some(
                AppendDecision::SimpleTail
                | AppendDecision::ParallelResult
                | AppendDecision::BranchContinuation,
            ) => {}
            None => return false,
        }
    }
    if let (Some(uuid_id), Some(active_cache_updates)) = (active_cache_update, active_cache_updates)
    {
        active_cache_updates.push(uuid_id);
    }
    true
}
