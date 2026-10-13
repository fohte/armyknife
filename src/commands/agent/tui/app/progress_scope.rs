use std::collections::HashSet;

use super::super::progress_fetch::{direct_delegates, task_descendant_delegates};
use super::App;
use crate::commands::agent::types::Session;

pub(super) fn is_project_manager(session: &Session) -> bool {
    session.work_type.as_deref() == Some("project-manager")
}

pub(super) struct ProgressScope {
    pub(super) work_sessions: Vec<Session>,
    pub(super) delegated: Vec<Session>,
}

impl App {
    pub(super) fn session_by_id(&self, session_id: &str) -> Option<Session> {
        self.sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .cloned()
    }

    pub(in crate::commands::agent::tui) fn progress_task_session_ids(
        &self,
        task_id: &str,
    ) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|session| {
                self.task_by_session
                    .get(&session.session_id)
                    .is_some_and(|task| task.task_id == task_id)
                    || self
                        .tq_snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.session_tasks.get(&session.session_id))
                        .is_some_and(|tasks| tasks.iter().any(|task| task.task_id == task_id))
            })
            .map(|session| session.session_id.clone())
            .collect()
    }

    pub(in crate::commands::agent::tui) fn progress_fetch_task_scope(
        &self,
        session_id: &str,
    ) -> (Option<String>, Option<Vec<String>>) {
        if self
            .session_by_id(session_id)
            .is_some_and(|session| is_project_manager(&session))
        {
            return (None, Some(Vec::new()));
        }

        let task_id = self
            .task_by_session
            .get(session_id)
            .map(|task| task.task_id.clone());
        let task_session_ids = task_id
            .as_deref()
            .map(|task_id| self.progress_task_session_ids(task_id));

        (task_id, task_session_ids)
    }

    pub(super) fn progress_scope(&self, selected: &Session) -> ProgressScope {
        if is_project_manager(selected) {
            return ProgressScope {
                work_sessions: vec![selected.clone()],
                delegated: Vec::new(),
            };
        }

        let Some(task_id) = self
            .task_by_session
            .get(&selected.session_id)
            .map(|task| task.task_id.as_str())
        else {
            return ProgressScope {
                work_sessions: vec![selected.clone()],
                delegated: direct_delegates(self.sessions.clone(), &selected.session_id),
            };
        };

        let mut task_session_ids = self
            .progress_task_session_ids(task_id)
            .into_iter()
            .collect::<HashSet<_>>();
        task_session_ids.insert(selected.session_id.clone());
        let work_sessions = self
            .sessions
            .iter()
            .filter(|session| task_session_ids.contains(&session.session_id))
            .cloned()
            .collect::<Vec<_>>();

        ProgressScope {
            work_sessions,
            delegated: task_descendant_delegates(&self.sessions, &task_session_ids),
        }
    }
}
