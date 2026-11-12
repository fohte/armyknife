use chrono::{DateTime, Utc};

use super::super::super::ChatItem;
use super::super::super::proposals::ProposalItem;
use super::super::time_placed::{TimePlaced, TimePlacedTimeline};

pub(super) type ProposalTimeline<'a> = TimePlacedTimeline<'a, ProposalItem>;

impl TimePlaced for ProposalItem {
    fn seq(&self) -> i64 {
        self.seq
    }

    fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    fn chat_item(&self) -> ChatItem {
        self.item.clone()
    }
}
