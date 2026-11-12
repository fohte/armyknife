use chrono::{DateTime, Utc};

use super::time_placed::{TimePlaced, TimePlacedTimeline};
use crate::commands::agent::store::AgentTodo;
use crate::commands::agent::watch_web::messages::ChatItem;

pub(super) type TodoTimeline<'a> = TimePlacedTimeline<'a, AgentTodo>;

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
            comment: self.comment.clone(),
        }
    }
}
