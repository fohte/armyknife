use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use super::super::ChatItem;
use super::ToolSummary;
use crate::commands::agent::store::AgentDecision;

pub(super) struct DecisionTimeline {
    decisions: BTreeMap<i64, AgentDecision>,
    ordered: Vec<(DateTime<Utc>, i64)>,
    next: usize,
}

impl DecisionTimeline {
    pub(super) fn new(decisions: &[AgentDecision]) -> Self {
        let decisions = decisions
            .iter()
            .cloned()
            .map(|decision| (decision.seq, decision))
            .collect::<BTreeMap<_, _>>();
        let mut ordered = decisions
            .values()
            .filter_map(|decision| {
                DateTime::parse_from_rfc3339(&decision.created_at)
                    .ok()
                    .map(|created_at| (created_at.with_timezone(&Utc), decision.seq))
            })
            .collect::<Vec<_>>();
        ordered.sort_unstable();

        Self {
            decisions,
            ordered,
            next: 0,
        }
    }

    pub(super) fn insert_before_record(
        &mut self,
        record_time: Option<DateTime<Utc>>,
        tools: &mut ToolSummary,
        items: &mut Vec<ChatItem>,
    ) {
        let Some(record_time) = record_time else {
            return;
        };
        if !self
            .ordered
            .get(self.next)
            .is_some_and(|(created_at, _)| *created_at < record_time)
        {
            return;
        }

        tools.flush_into(items);
        while let Some((created_at, seq)) = self.ordered.get(self.next)
            && *created_at < record_time
        {
            if let Some(decision) = self.decisions.remove(seq) {
                items.push(decision.into());
            }
            self.next += 1;
        }
    }

    pub(super) fn append_remaining(self, items: &mut Vec<ChatItem>) {
        items.extend(self.decisions.into_values().map(ChatItem::from));
    }
}

pub(super) fn insert_time_placed_decisions(
    record_time: Option<DateTime<Utc>>,
    timeline: &mut DecisionTimeline,
    tools: &mut ToolSummary,
    items: &mut Vec<ChatItem>,
) {
    timeline.insert_before_record(record_time, tools, items);
}
