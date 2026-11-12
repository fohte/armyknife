use std::collections::HashSet;

use chrono::{DateTime, Utc};

use super::super::ChatItem;
use super::ToolSummary;

pub(super) trait TimePlaced {
    fn seq(&self) -> i64;
    fn created_at(&self) -> DateTime<Utc>;
    fn chat_item(&self) -> ChatItem;
}

pub(super) struct TimePlacedTimeline<'a, T> {
    entries: &'a [T],
    inserted_seqs: HashSet<i64>,
}

impl<'a, T: TimePlaced> TimePlacedTimeline<'a, T> {
    pub(super) fn new(entries: &'a [T]) -> Self {
        Self {
            entries,
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
            .entries
            .iter()
            .filter(|entry| {
                entry.created_at() <= timestamp && !self.inserted_seqs.contains(&entry.seq())
            })
            .collect::<Vec<_>>();
        due.sort_by_key(|entry| (entry.created_at(), entry.seq()));
        if due.is_empty() {
            return;
        }

        tools.flush_into(items);
        for entry in due {
            items.push(entry.chat_item());
            self.inserted_seqs.insert(entry.seq());
        }
    }

    pub(super) fn append_remaining(&mut self, items: &mut Vec<ChatItem>) {
        let mut remaining = self
            .entries
            .iter()
            .filter(|entry| !self.inserted_seqs.contains(&entry.seq()))
            .collect::<Vec<_>>();
        remaining.sort_by_key(|entry| (entry.created_at(), entry.seq()));
        items.extend(remaining.into_iter().map(TimePlaced::chat_item));
    }
}
