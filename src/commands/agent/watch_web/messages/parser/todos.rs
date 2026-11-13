use chrono::{DateTime, Utc};

use super::time_placed::{TimePlaced, TimePlacedTimeline};
use crate::commands::agent::store::AgentTodo;
use crate::commands::agent::watch_web::messages::ChatItem;

#[derive(Clone)]
pub(super) struct TodoTimeline {
    completed: TimePlacedTimeline<AgentTodo>,
    pending: Vec<AgentTodo>,
}

impl TodoTimeline {
    pub(super) fn new(todos: &[AgentTodo]) -> Self {
        let completed = todos
            .iter()
            .filter(|todo| todo.completed_at.is_some())
            .cloned()
            .collect::<Vec<_>>();
        let pending = todos
            .iter()
            .filter(|todo| todo.completed_at.is_none())
            .cloned()
            .collect();

        Self {
            completed: TimePlacedTimeline::new(&completed),
            pending,
        }
    }

    pub(super) fn insert_before_record(
        &mut self,
        timestamp: Option<DateTime<Utc>>,
        items: &mut Vec<ChatItem>,
        tools: &mut super::ToolSummary,
    ) {
        self.completed.insert_before_record(timestamp, items, tools);
    }

    pub(super) fn append_remaining(&mut self, items: &mut Vec<ChatItem>) {
        self.completed.append_remaining(items);
        let mut pending = self.pending.iter().collect::<Vec<_>>();
        pending.sort_by_key(|todo| (todo.created_at, todo.seq));
        items.extend(pending.into_iter().map(TimePlaced::chat_item));
    }
}

impl TimePlaced for AgentTodo {
    fn seq(&self) -> i64 {
        self.seq
    }

    fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    fn chat_item(&self) -> ChatItem {
        ChatItem::Todo {
            seq: self.seq,
            title: self.title.clone(),
            detail: self.detail.clone(),
            completed: self.completed_at.is_some(),
            resolution: self.resolution,
            comment: self.comment.clone(),
        }
    }
}
