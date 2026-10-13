use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::session_rows::SessionTask;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TqSnapshot {
    pub format_version: u8,
    pub fetched_at: DateTime<Utc>,
    pub session_tasks: HashMap<String, Vec<SessionTask>>,
}

impl TqSnapshot {
    pub(super) fn new(session_tasks: HashMap<String, Vec<SessionTask>>) -> Self {
        Self {
            format_version: 1,
            fetched_at: Utc::now(),
            session_tasks,
        }
    }

    pub(super) fn from_legacy(
        session_tasks: HashMap<String, SessionTask>,
        fetched_at: DateTime<Utc>,
    ) -> Self {
        Self {
            format_version: 1,
            fetched_at,
            session_tasks: session_tasks
                .into_iter()
                .map(|(session_id, task)| (session_id, vec![task]))
                .collect(),
        }
    }
}
