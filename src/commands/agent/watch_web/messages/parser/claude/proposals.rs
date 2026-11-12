use std::collections::HashSet;

use chrono::{DateTime, Utc};

use super::super::super::ChatItem;
use super::super::super::proposals::ProposalItem;
use super::super::ToolSummary;

#[derive(Clone)]
pub(super) struct ProposalTimeline {
    proposals: Vec<ProposalItem>,
    inserted_seqs: HashSet<i64>,
}

impl ProposalTimeline {
    pub(super) fn new(proposals: &[ProposalItem]) -> Self {
        Self {
            proposals: proposals.to_vec(),
            inserted_seqs: HashSet::new(),
        }
    }

    pub(super) fn insert_before_record(
        &mut self,
        timestamp: Option<DateTime<Utc>>,
        items: &mut Vec<ChatItem>,
        tools: &mut ToolSummary,
    ) {
        let Some(timestamp) = timestamp else {
            return;
        };
        let mut due = self
            .proposals
            .iter()
            .filter(|proposal| {
                proposal.created_at <= timestamp && !self.inserted_seqs.contains(&proposal.seq)
            })
            .collect::<Vec<_>>();
        due.sort_by_key(|proposal| (proposal.created_at, proposal.seq));
        if due.is_empty() {
            return;
        }

        tools.flush_into(items);
        for proposal in due {
            items.push(proposal.item.clone());
            self.inserted_seqs.insert(proposal.seq);
        }
    }

    pub(super) fn append_remaining(&mut self, items: &mut Vec<ChatItem>) {
        let mut remaining = self
            .proposals
            .iter()
            .filter(|proposal| !self.inserted_seqs.contains(&proposal.seq))
            .collect::<Vec<_>>();
        remaining.sort_by_key(|proposal| (proposal.created_at, proposal.seq));
        items.extend(remaining.into_iter().map(|proposal| proposal.item.clone()));
    }
}
