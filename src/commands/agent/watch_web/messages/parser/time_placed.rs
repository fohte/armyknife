use std::collections::HashSet;

use chrono::{DateTime, Utc};

use super::super::ChatItem;
use super::ToolSummary;
use crate::commands::agent::store::WatchWebPeerOutMessage;

pub(super) trait TimePlaced {
    fn seq(&self) -> i64;
    fn created_at(&self) -> DateTime<Utc>;
    fn chat_item(&self) -> ChatItem;
}

impl TimePlaced for WatchWebPeerOutMessage {
    fn seq(&self) -> i64 {
        self.id
    }

    fn created_at(&self) -> DateTime<Utc> {
        self.sent_at
    }

    fn chat_item(&self) -> ChatItem {
        ChatItem::Peer {
            direction: super::super::PeerDirection::To,
            session_id: self.to_session_id.clone(),
            session_title: None,
            content: vec![super::super::UserContentPart::Text {
                text: self.content.clone(),
            }],
            timestamp: Some(self.sent_at.to_rfc3339()),
            failed: self.failed,
        }
    }
}

#[derive(Clone)]
pub(super) struct TimePlacedTimeline<T> {
    entries: Vec<T>,
    inserted_seqs: HashSet<i64>,
}

impl<T: TimePlaced + Clone> TimePlacedTimeline<T> {
    pub(super) fn new(entries: &[T]) -> Self {
        Self {
            entries: entries.to_vec(),
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
