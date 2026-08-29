//! Fetches which locally known Claude Code sessions are linked to a tq
//! task, keyed by session_id, for the session list's title-prefix renderer.
//! Pure read-only against tq; never touches local session state.

use std::collections::{HashMap, HashSet};

use super::session_rows::SessionTask;
use crate::commands::cc::claude_sessions::normalize_title;
use crate::infra::tq::{SessionTasks, TqClient};

/// Fetches tq's session -> tasks listing and reduces it to one
/// [`SessionTask`] per locally known session_id.
///
/// Returns `Ok(HashMap::new())`, not an error, when tq integration isn't
/// available (`client` is `None`, i.e. [`TqClient::detect`] found no `tq`
/// binary on `PATH`) -- the caller treats "not available" and "fetched,
/// nothing linked" identically: sessions render with no title-prefix.
pub async fn fetch_session_tasks(
    client: Option<TqClient>,
    local_session_ids: HashSet<String>,
) -> Result<HashMap<String, SessionTask>, String> {
    let Some(client) = client else {
        return Ok(HashMap::new());
    };

    let sessions = client
        .list_session_tasks()
        .await
        .map_err(|e| e.to_string())?;

    Ok(build_task_by_session(sessions, &local_session_ids))
}

/// Reduces tq's session -> tasks listing to one [`SessionTask`] per locally
/// known session_id. A session linked to multiple tasks keeps only the
/// first (tq's own ordering) -- the title-prefix has room for exactly one
/// task, and this PR has no UI for showing more.
fn build_task_by_session(
    sessions: Vec<SessionTasks>,
    local_session_ids: &HashSet<String>,
) -> HashMap<String, SessionTask> {
    sessions
        .into_iter()
        .filter(|session| local_session_ids.contains(&session.session_id))
        .filter_map(|session| {
            let task = session.tasks.into_iter().next()?;
            Some((
                session.session_id,
                SessionTask {
                    task_id: task.id,
                    task_number: task.number,
                    task_title: normalize_title(&task.title),
                    parent_task_id: task.parent_id,
                },
            ))
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
        }
    }

    fn session(session_id: &str, tasks: Vec<TqTask>) -> SessionTasks {
        SessionTasks {
            session_id: session_id.to_string(),
            tasks,
        }
    }

    #[tokio::test]
    async fn client_none_returns_empty_without_spawning_tq() {
        let result = fetch_session_tasks(None, ids(&["session-1"])).await;

        assert_eq!(result, Ok(HashMap::new()));
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
            },
        )]),
    )]
    #[case::session_with_no_linked_tasks_is_absent(
        vec![session("session-a", vec![])],
        &["session-a"],
        HashMap::new(),
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
