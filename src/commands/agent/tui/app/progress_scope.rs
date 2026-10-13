use std::collections::HashSet;

use super::super::progress::ProgressFetchScope;
use super::super::progress_fetch::{direct_delegates, task_descendant_delegates};
use super::App;
use crate::commands::agent::types::Session;

pub(super) fn is_project_manager(session: &Session) -> bool {
    session.work_type.as_deref() == Some("project-manager")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::commands::agent::tui) enum ProgressScopeKind {
    OwnOnly,
    Direct,
    Task {
        task_id: String,
        session_ids: Vec<String>,
    },
}

impl ProgressScopeKind {
    pub(super) fn includes_fetched_progress(&self) -> bool {
        !matches!(self, Self::OwnOnly)
    }

    pub(in crate::commands::agent::tui) fn fetch_scope(&self) -> Option<ProgressFetchScope> {
        match self {
            Self::OwnOnly => None,
            Self::Direct => Some(ProgressFetchScope {
                task_id: None,
                session_ids: None,
            }),
            Self::Task {
                task_id,
                session_ids,
            } => Some(ProgressFetchScope {
                task_id: Some(task_id.clone()),
                session_ids: Some(session_ids.clone()),
            }),
        }
    }
}

pub(super) struct ProgressScope {
    pub(super) kind: ProgressScopeKind,
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

    pub(in crate::commands::agent::tui) fn progress_scope_for_session(
        &self,
        session_id: &str,
    ) -> Option<ProgressScopeKind> {
        let selected = self.session_by_id(session_id)?;
        Some(self.progress_scope(&selected).kind)
    }

    pub(super) fn progress_scope(&self, selected: &Session) -> ProgressScope {
        if is_project_manager(selected) {
            return ProgressScope {
                kind: ProgressScopeKind::OwnOnly,
                work_sessions: vec![selected.clone()],
                delegated: Vec::new(),
            };
        }

        let Some(task_id) = self
            .task_by_session
            .get(&selected.session_id)
            .map(|task| task.task_id.clone())
        else {
            return ProgressScope {
                kind: ProgressScopeKind::Direct,
                work_sessions: vec![selected.clone()],
                delegated: direct_delegates(self.sessions.clone(), &selected.session_id),
            };
        };

        let mut task_session_ids = self.progress_task_session_ids(&task_id);
        task_session_ids.sort_unstable();
        let mut task_session_ids_set = task_session_ids.iter().cloned().collect::<HashSet<_>>();
        task_session_ids_set.insert(selected.session_id.clone());
        let work_sessions = self
            .sessions
            .iter()
            .filter(|session| {
                !session.is_delegated && task_session_ids_set.contains(&session.session_id)
            })
            .cloned()
            .collect::<Vec<_>>();

        ProgressScope {
            kind: ProgressScopeKind::Task {
                task_id,
                session_ids: task_session_ids,
            },
            work_sessions,
            delegated: task_descendant_delegates(&self.sessions, &task_session_ids_set),
        }
    }
}
