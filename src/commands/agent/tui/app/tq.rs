use std::collections::HashSet;

use super::App;
use crate::commands::agent::tui::tq_snapshot::TqSnapshot;

impl App {
    pub fn is_tq_loading(&self) -> bool {
        self.tq_snapshot.is_none() && self.tq_refreshing
    }

    pub fn set_tq_snapshot(&mut self, mut snapshot: TqSnapshot) {
        let session_ids = self
            .sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<HashSet<_>>();
        snapshot
            .session_tasks
            .retain(|session_id, _| session_ids.contains(session_id.as_str()));
        self.task_by_session = snapshot
            .session_tasks
            .iter()
            .filter_map(|(session_id, tasks)| {
                tasks
                    .first()
                    .cloned()
                    .map(|task| (session_id.clone(), task))
            })
            .collect();
        self.tq_snapshot = Some(snapshot);
    }

    pub fn tq_refresh_started(&mut self) {
        self.tq_refreshing = true;
    }

    pub fn tq_refresh_finished(&mut self) {
        self.tq_refreshing = false;
    }
}
