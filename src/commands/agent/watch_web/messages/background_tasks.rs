use anyhow::Result;

use super::{BackgroundTask, BackgroundTaskKind};
use crate::commands::agent::bg_tasks;

pub(super) fn registered_background_tasks(session_id: &str) -> Result<Vec<BackgroundTask>> {
    Ok(bg_tasks::pending_tasks(session_id)?
        .into_iter()
        .map(|task| BackgroundTask {
            id: task.id,
            description: task.description,
            elapsed_seconds: task.elapsed_seconds,
            kind: BackgroundTaskKind::BackgroundTask,
            output_path: Some(task.stdout_path),
            agent_type: None,
        })
        .collect())
}
