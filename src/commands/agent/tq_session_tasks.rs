use std::collections::{HashMap, HashSet};
use std::future::Future;

use crate::commands::agent::claude_sessions::normalize_title;
use crate::infra::tq::{SessionTasks, TqClient, TqTaskStatus};

/// A tq task linked to a session, as needed by session list views. `parent_task_id`
/// is `None` both when the task has no parent and when an older `tq` binary
/// cannot report it, so callers degrade to matching the same task only.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub(super) struct SessionTask {
    pub task_id: String,
    pub task_number: u32,
    pub task_title: String,
    pub parent_task_id: Option<String>,
    pub is_closed: bool,
}

/// Fetches tq links for local sessions and their missing parent sessions.
pub(super) async fn fetch_session_tasks_with_parents(
    client: &TqClient,
    local_session_ids: &HashSet<String>,
) -> Result<Vec<SessionTasks>, String> {
    if local_session_ids.is_empty() {
        return Ok(Vec::new());
    }

    let sessions = client
        .list_session_tasks(local_session_ids)
        .await
        .map_err(|error| error.to_string())?;
    let parent_sessions = fetch_missing_parent_sessions(
        &sessions,
        local_session_ids,
        |parent_session_ids| async move { client.list_session_tasks(&parent_session_ids).await },
    )
    .await
    .map_err(|error| error.to_string())?;

    let mut sessions = sessions;
    sessions.extend(parent_sessions);
    Ok(sessions)
}

fn parent_session_ids(
    sessions: &[SessionTasks],
    local_session_ids: &HashSet<String>,
) -> HashSet<String> {
    sessions
        .iter()
        .filter(|session| local_session_ids.contains(&session.session_id))
        .filter_map(|session| session.parent_session_id.clone())
        .collect()
}

fn missing_parent_session_ids(
    sessions: &[SessionTasks],
    local_session_ids: &HashSet<String>,
) -> HashSet<String> {
    let returned_session_ids = sessions
        .iter()
        .map(|session| session.session_id.clone())
        .collect::<HashSet<_>>();
    parent_session_ids(sessions, local_session_ids)
        .difference(&returned_session_ids)
        .cloned()
        .collect()
}

pub(super) async fn fetch_missing_parent_sessions<F, Fut, E>(
    sessions: &[SessionTasks],
    local_session_ids: &HashSet<String>,
    fetch: F,
) -> Result<Vec<SessionTasks>, E>
where
    F: FnOnce(HashSet<String>) -> Fut,
    Fut: Future<Output = Result<Vec<SessionTasks>, E>>,
{
    let missing_parent_session_ids = missing_parent_session_ids(sessions, local_session_ids);
    if missing_parent_session_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut parent_sessions = fetch(missing_parent_session_ids.clone()).await?;
    parent_sessions.retain(|session| missing_parent_session_ids.contains(&session.session_id));
    Ok(parent_sessions)
}

/// Reduces tq's session -> tasks listing to one task per locally known session.
/// It prefers session-specific tasks, then open tasks, then newer links.
pub(super) fn build_task_by_session(
    sessions: Vec<SessionTasks>,
    local_session_ids: &HashSet<String>,
) -> HashMap<String, SessionTask> {
    build_all_tasks_by_session(sessions, local_session_ids)
        .into_iter()
        .filter_map(|(session_id, tasks)| {
            let task = tasks.into_iter().next()?;
            Some((session_id, task))
        })
        .collect()
}

/// Builds the task order shared by the TUI and Web UI. Filtering by local IDs
/// also handles older `tq` binaries that ignore the session ID argument.
pub(super) fn build_all_tasks_by_session(
    sessions: Vec<SessionTasks>,
    local_session_ids: &HashSet<String>,
) -> HashMap<String, Vec<SessionTask>> {
    let referenced_parent_session_ids = parent_session_ids(&sessions, local_session_ids);
    let task_ids_by_parent_session = sessions
        .iter()
        .filter(|session| referenced_parent_session_ids.contains(&session.session_id))
        .map(|session| {
            (
                session.session_id.clone(),
                session
                    .tasks
                    .iter()
                    .map(|task| task.id.clone())
                    .collect::<HashSet<_>>(),
            )
        })
        .collect::<HashMap<_, _>>();

    sessions
        .into_iter()
        .filter(|session| local_session_ids.contains(&session.session_id))
        .map(|session| {
            let inherited_task_ids = session
                .parent_session_id
                .as_ref()
                .and_then(|parent_id| task_ids_by_parent_session.get(parent_id));
            let mut tasks = session.tasks;
            tasks.sort_by(|left, right| {
                let inherited_order = inherited_task_ids.map_or(std::cmp::Ordering::Equal, |ids| {
                    ids.contains(&left.id).cmp(&ids.contains(&right.id))
                });
                inherited_order
                    .then_with(|| {
                        (left.status == TqTaskStatus::Completed)
                            .cmp(&(right.status == TqTaskStatus::Completed))
                    })
                    .then_with(|| right.linked_at.cmp(&left.linked_at))
            });
            let tasks = tasks
                .into_iter()
                .map(|task| SessionTask {
                    task_id: task.id,
                    task_number: task.number,
                    task_title: normalize_title(&task.title),
                    parent_task_id: task.parent_id,
                    is_closed: task.status == TqTaskStatus::Completed,
                })
                .collect();
            (session.session_id, tasks)
        })
        .collect()
}
