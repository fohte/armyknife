use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};

use super::super::super::ChatItem;
use super::super::super::proposals::ProposalItem;
use super::super::ToolSummary;

pub(super) struct ProposalTimeline<'a> {
    proposals: &'a [ProposalItem],
    timed_seqs: HashSet<i64>,
    transcript_seqs: HashSet<i64>,
}

impl<'a> ProposalTimeline<'a> {
    pub(super) fn new(proposals: &'a [ProposalItem]) -> Self {
        Self {
            proposals,
            timed_seqs: HashSet::new(),
            transcript_seqs: HashSet::new(),
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
                proposal.created_at <= timestamp
                    && !self.transcript_seqs.contains(&proposal.seq)
                    && !self.timed_seqs.contains(&proposal.seq)
            })
            .collect::<Vec<_>>();
        due.sort_by_key(|proposal| (proposal.created_at, proposal.seq));
        if due.is_empty() {
            return;
        }

        tools.flush_into(items);
        for proposal in due {
            items.push(proposal.item.clone());
            self.timed_seqs.insert(proposal.seq);
        }
    }

    pub(super) fn move_to_transcript_position(
        &mut self,
        seq: i64,
        items: &mut Vec<ChatItem>,
        question_uses: &mut HashMap<String, Vec<usize>>,
    ) {
        if self.timed_seqs.remove(&seq)
            && let Some(index) = items.iter().position(
                |item| matches!(item, ChatItem::Proposal { seq: item_seq, .. } if *item_seq == seq),
            )
        {
            items.remove(index);
            for indices in question_uses.values_mut() {
                for question_index in indices {
                    if *question_index > index {
                        *question_index -= 1;
                    }
                }
            }
        }
        self.transcript_seqs.insert(seq);
    }

    pub(super) fn append_remaining(&mut self, items: &mut Vec<ChatItem>) {
        let mut remaining = self
            .proposals
            .iter()
            .filter(|proposal| {
                !self.transcript_seqs.contains(&proposal.seq)
                    && !self.timed_seqs.contains(&proposal.seq)
            })
            .collect::<Vec<_>>();
        remaining.sort_by_key(|proposal| (proposal.created_at, proposal.seq));
        items.extend(remaining.into_iter().map(|proposal| proposal.item.clone()));
    }
}
