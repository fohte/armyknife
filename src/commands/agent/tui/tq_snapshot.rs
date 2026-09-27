use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::session_rows::SessionTask;
use crate::infra::tq::{TqProject, TqSidebarTask, TqTaskStatus};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TqSnapshot {
    pub format_version: u8,
    pub fetched_at: DateTime<Utc>,
    pub session_tasks: HashMap<String, Vec<SessionTask>>,
    pub tasks: HashMap<String, TqSidebarTask>,
    pub projects: HashMap<String, TqProject>,
}

impl TqSnapshot {
    pub(super) fn new(
        session_tasks: HashMap<String, Vec<SessionTask>>,
        tasks: Vec<TqSidebarTask>,
        projects: Vec<TqProject>,
    ) -> Self {
        Self {
            format_version: 1,
            fetched_at: Utc::now(),
            session_tasks,
            tasks: tasks
                .into_iter()
                .map(|task| (task.id.clone(), task))
                .collect(),
            projects: projects
                .into_iter()
                .map(|project| (project.id.clone(), project))
                .collect(),
        }
    }

    pub(super) fn from_legacy(
        session_tasks: HashMap<String, SessionTask>,
        fetched_at: DateTime<Utc>,
    ) -> Self {
        let mut snapshot = Self {
            format_version: 1,
            fetched_at,
            session_tasks: HashMap::new(),
            tasks: HashMap::new(),
            projects: HashMap::new(),
        };

        for (session_id, task) in session_tasks {
            snapshot
                .tasks
                .entry(task.task_id.clone())
                .or_insert_with(|| TqSidebarTask {
                    id: task.task_id.clone(),
                    number: task.task_number,
                    title: task.task_title.clone(),
                    status: if task.is_closed {
                        TqTaskStatus::Completed
                    } else {
                        TqTaskStatus::Todo
                    },
                    status_reason: task.is_closed.then(|| "completed".to_string()),
                    commitment: None,
                    due_date: None,
                    parent_id: task.parent_task_id.clone(),
                    project_id: None,
                    created_at: None,
                });
            snapshot.session_tasks.insert(session_id, vec![task]);
        }

        snapshot
    }
}
