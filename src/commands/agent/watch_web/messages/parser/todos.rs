use std::collections::HashSet;

use chrono::{DateTime, Utc};

use super::super::ChatItem;
use super::ToolSummary;
use crate::commands::agent::store::AgentTodo;

pub(super) struct TodoTimeline<'a> {
    todos: &'a [AgentTodo],
    inserted_seqs: HashSet<i64>,
}

impl<'a> TodoTimeline<'a> {
    pub(super) fn new(todos: &'a [AgentTodo]) -> Self {
        Self {
            todos,
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
            .todos
            .iter()
            .filter(|todo| todo.created_at <= timestamp && !self.inserted_seqs.contains(&todo.seq))
            .collect::<Vec<_>>();
        due.sort_by_key(|todo| (todo.created_at, todo.seq));
        if due.is_empty() {
            return;
        }

        tools.flush_into(items);
        for todo in due {
            items.push(todo_chat_item(todo));
            self.inserted_seqs.insert(todo.seq);
        }
    }

    pub(super) fn append_remaining(&mut self, items: &mut Vec<ChatItem>) {
        let mut remaining = self
            .todos
            .iter()
            .filter(|todo| !self.inserted_seqs.contains(&todo.seq))
            .collect::<Vec<_>>();
        remaining.sort_by_key(|todo| (todo.created_at, todo.seq));
        items.extend(remaining.into_iter().map(todo_chat_item));
    }
}

fn todo_chat_item(todo: &AgentTodo) -> ChatItem {
    ChatItem::Todo {
        seq: todo.seq,
        title: todo.title.clone(),
        detail: todo.detail.clone(),
        completed: todo.completed_at.is_some(),
        comment: todo.comment.clone(),
    }
}
