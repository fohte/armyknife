use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use super::super::super::ChatItem;
use super::super::ToolSummary;
use crate::commands::agent::store::AgentDecision;

pub(in crate::commands::agent::watch_web::messages::parser) fn insert_time_placed_decisions(
    record_time: Option<DateTime<Utc>>,
    unplaced_decisions: &mut BTreeMap<i64, AgentDecision>,
    tools: &mut ToolSummary,
    items: &mut Vec<ChatItem>,
) {
    let Some(record_time) = record_time else {
        return;
    };
    let mut decisions_to_place = unplaced_decisions
        .values()
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
