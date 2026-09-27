//! Fetches which locally known Claude Code sessions are linked to a tq
//! task, keyed by session_id, for the session list's task-number column.
//! Pure read-only against tq; never touches local session state.

use std::collections::{HashMap, HashSet};

use super::session_rows::SessionTask;
use super::tq_snapshot::TqSnapshot;
use crate::commands::agent::claude_sessions::normalize_title;
use crate::infra::tq::{SessionTasks, TqClient, TqTaskStatus};

/// Fetches tq's session -> tasks listing and reduces it to one
/// [`SessionTask`] per locally known session_id.
///
/// Returns `Ok(None)` when the fetch is skipped because tq is unavailable or
/// there are no local sessions. A successful lookup returns `Some`, including
/// an empty map when no tasks are linked, so callers can retain cached links
/// when no fresh result is available.
#[cfg(test)]
pub async fn fetch_session_tasks(
    client: Option<TqClient>,
    local_session_ids: HashSet<String>,
) -> Result<Option<HashMap<String, SessionTask>>, String> {
    let Some(client) = client else {
        return Ok(None);
    };
    if local_session_ids.is_empty() {
        // Skip the round trip: tq's --session-id filter needs at least one
        // id, and an empty request would otherwise return its full history.
        return Ok(None);
    }

    let sessions = client
        .list_session_tasks(&local_session_ids)
        .await
        .map_err(|e| e.to_string())?;

    Ok(Some(build_task_by_session(sessions, &local_session_ids)))
}

/// Fetches the linked task IDs, their full task records and all projects as
/// one snapshot for the sidebar and the session task-number column.
pub async fn fetch_sidebar_snapshot(
    client: Option<TqClient>,
    local_session_ids: HashSet<String>,
) -> Result<Option<TqSnapshot>, String> {
    if local_session_ids.is_empty() {
        return Ok(None);
    }
    let Some(client) = client else {
        return Ok(None);
    };

    let sessions = client
        .list_session_tasks(&local_session_ids)
        .await
        .map_err(|e| e.to_string())?;
    let session_tasks = build_all_tasks_by_session(sessions, &local_session_ids);
    let linked_task_ids = session_tasks
        .values()
        .flat_map(|tasks| tasks.iter().map(|task| task.task_id.clone()))
        .collect::<HashSet<_>>();

    let (tasks, projects) = if linked_task_ids.is_empty() {
        (
            Vec::new(),
            client.list_projects().await.map_err(|e| e.to_string())?,
        )
    } else {
        tokio::try_join!(
            client.list_sidebar_tasks(&linked_task_ids),
            client.list_projects(),
        )
        .map_err(|e| e.to_string())?
    };

    Ok(Some(TqSnapshot::new(session_tasks, tasks, projects)))
}

/// Reduces tq's session -> tasks listing to one [`SessionTask`] per locally
/// known session_id. A session linked to multiple tasks keeps only the
/// first (tq's own ordering) -- the task-number column only has room for one.
///
/// The `local_session_ids` filter here is also the fallback for a `tq`
/// binary predating `--session-id`, which silently ignores the flag and
/// returns every session it knows about.
#[cfg(test)]
fn build_task_by_session(
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

fn build_all_tasks_by_session(
    sessions: Vec<SessionTasks>,
    local_session_ids: &HashSet<String>,
) -> HashMap<String, Vec<SessionTask>> {
    sessions
        .into_iter()
        .filter(|session| local_session_ids.contains(&session.session_id))
        .map(|session| {
            let tasks = session
                .tasks
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

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::infra::tq::TqTask;

    fn ids(values: &[&str]) -> HashSet<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    fn task(id: &str, number: u32, title: &str, parent_id: Option<&str>) -> TqTask {
        TqTask {
            id: id.to_string(),
            number,
            title: title.to_string(),
            parent_id: parent_id.map(String::from),
            status: TqTaskStatus::Todo,
        }
    }

    fn closed_task(id: &str, number: u32, title: &str, parent_id: Option<&str>) -> TqTask {
        TqTask {
            status: TqTaskStatus::Completed,
            ..task(id, number, title, parent_id)
        }
    }

    fn session(session_id: &str, tasks: Vec<TqTask>) -> SessionTasks {
        SessionTasks {
            session_id: session_id.to_string(),
            tasks,
        }
    }

    #[tokio::test]
    async fn client_none_skips_without_spawning_tq() {
        let result = fetch_session_tasks(None, ids(&["session-1"])).await;

        assert_eq!(result, Ok(None));
    }

    #[tokio::test]
    async fn empty_local_session_ids_skips_without_spawning_tq() {
        let result = fetch_session_tasks(Some(TqClient), HashSet::new()).await;

        assert_eq!(result, Ok(None));
    }

    #[rstest]
    #[case::maps_linked_sessions_to_their_task(
        vec![
            session("session-a", vec![task("task-1", 10, "First task", None)]),
            session("session-b", vec![]),
        ],
        &["session-a", "session-b"],
        HashMap::from([(
            "session-a".to_string(),
            SessionTask {
                task_id: "task-1".to_string(),
                task_number: 10,
                task_title: "First task".to_string(),
                parent_task_id: None,
                is_closed: false,
            },
        )]),
    )]
    #[case::drops_sessions_outside_the_local_set(
        vec![
            session("session-a", vec![task("task-1", 1, "Task", None)]),
            session("remote-session", vec![task("task-1", 1, "Task", None)]),
        ],
        &["session-a"],
        HashMap::from([(
            "session-a".to_string(),
            SessionTask {
                task_id: "task-1".to_string(),
                task_number: 1,
                task_title: "Task".to_string(),
                parent_task_id: None,
                is_closed: false,
            },
        )]),
    )]
    #[case::session_linked_to_multiple_tasks_keeps_only_the_first(
        vec![session(
            "session-a",
            vec![
                task("task-1", 1, "Task one", None),
                task("task-2", 2, "Task two", None),
            ],
        )],
        &["session-a"],
        HashMap::from([(
            "session-a".to_string(),
            SessionTask {
                task_id: "task-1".to_string(),
                task_number: 1,
                task_title: "Task one".to_string(),
                parent_task_id: None,
                is_closed: false,
            },
        )]),
    )]
    #[case::preserves_parent_task_id(
        vec![session(
            "session-a",
            vec![task("task-2", 2, "Child task", Some("task-1"))],
        )],
        &["session-a"],
        HashMap::from([(
            "session-a".to_string(),
            SessionTask {
                task_id: "task-2".to_string(),
                task_number: 2,
                task_title: "Child task".to_string(),
                parent_task_id: Some("task-1".to_string()),
                is_closed: false,
            },
        )]),
    )]
    #[case::session_with_no_linked_tasks_is_absent(
        vec![session("session-a", vec![])],
        &["session-a"],
        HashMap::new(),
    )]
    #[case::maps_completed_status_to_is_closed(
        vec![session(
            "session-a",
            vec![closed_task("task-1", 1, "Task", None)],
        )],
        &["session-a"],
        HashMap::from([(
            "session-a".to_string(),
            SessionTask {
                task_id: "task-1".to_string(),
                task_number: 1,
                task_title: "Task".to_string(),
                parent_task_id: None,
                is_closed: true,
            },
        )]),
    )]
    fn build_task_by_session_cases(
        #[case] sessions: Vec<SessionTasks>,
        #[case] local_session_ids: &[&str],
        #[case] expected: HashMap<String, SessionTask>,
    ) {
        let result = build_task_by_session(sessions, &ids(local_session_ids));

        assert_eq!(result, expected);
    }
}
