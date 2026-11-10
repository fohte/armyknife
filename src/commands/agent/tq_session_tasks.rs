use std::collections::{HashMap, HashSet};
use std::future::Future;

use crate::commands::agent::claude_sessions::normalize_title;
use crate::infra::tq::{SessionTasks, TqClient, TqTask, TqTaskStatus};

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
            let tasks = tasks.into_iter().map(to_session_task).collect();
            (session.session_id, tasks)
        })
        .collect()
}

fn to_session_task(task: TqTask) -> SessionTask {
    SessionTask {
        task_id: task.id,
        task_number: task.number,
        task_title: normalize_title(&task.title),
        parent_task_id: task.parent_id,
        is_closed: task.status == TqTaskStatus::Completed,
    }
}

#[cfg(test)]
mod tests {
    use super::SessionTask;
    use super::build_all_tasks_by_session;
    use crate::infra::tq::{SessionTasks, TqTask, TqTaskStatus};
    use chrono::{DateTime, Utc};
    use rstest::rstest;
    use std::collections::{HashMap, HashSet};

    fn task(
        id: &str,
        number: u32,
        title: &str,
        linked_at: Option<&str>,
        status: TqTaskStatus,
    ) -> TqTask {
        TqTask {
            id: id.to_string(),
            number,
            title: title.to_string(),
            linked_at: linked_at.map(|value| {
                DateTime::parse_from_rfc3339(value)
                    .expect("valid timestamp")
                    .with_timezone(&Utc)
            }),
            parent_id: None,
            status,
        }
    }

    fn task_with_parent(
        id: &str,
        number: u32,
        title: &str,
        parent_id: &str,
        status: TqTaskStatus,
    ) -> TqTask {
        TqTask {
            parent_id: Some(parent_id.to_string()),
            ..task(id, number, title, None, status)
        }
    }

    fn view_task(id: &str, number: u32, title: &str, is_closed: bool) -> SessionTask {
        SessionTask {
            task_id: id.to_string(),
            task_number: number,
            task_title: title.to_string(),
            parent_task_id: None,
            is_closed,
        }
    }

    fn session(
        session_id: &str,
        parent_session_id: Option<&str>,
        tasks: Vec<TqTask>,
    ) -> SessionTasks {
        SessionTasks {
            session_id: session_id.to_string(),
            parent_session_id: parent_session_id.map(String::from),
            tasks,
        }
    }

    #[rstest]
    #[case::open_links_precede_completed_links(
        vec![session("session-a", None, vec![
            task("completed-newest", 1, "Completed newest", Some("2026-01-03T00:00:00Z"), TqTaskStatus::Completed),
            task("open-oldest", 2, "Open oldest", Some("2026-01-01T00:00:00Z"), TqTaskStatus::Todo),
            task("open-newest", 3, "Open newest", Some("2026-01-02T00:00:00Z"), TqTaskStatus::Todo),
        ])],
        HashMap::from([("session-a".to_string(), vec![
            view_task("open-newest", 3, "Open newest", false),
            view_task("open-oldest", 2, "Open oldest", false),
            view_task("completed-newest", 1, "Completed newest", true),
        ])]),
    )]
    #[case::newest_link_wins_when_all_tasks_are_completed(
        vec![session("session-a", None, vec![
            task("older", 1, "Older", Some("2026-01-01T00:00:00Z"), TqTaskStatus::Completed),
            task("newer", 2, "Newer", Some("2026-01-02T00:00:00Z"), TqTaskStatus::Completed),
        ])],
        HashMap::from([("session-a".to_string(), vec![
            view_task("newer", 2, "Newer", true),
            view_task("older", 1, "Older", true),
        ])]),
    )]
    #[case::session_specific_task_precedes_inherited_task(
        vec![
            session("child-session", Some("parent-session"), vec![
                task("inherited", 1, "Inherited task", Some("2026-01-02T00:00:00Z"), TqTaskStatus::Todo),
                task("session-specific", 2, "Session task", Some("2026-01-01T00:00:00Z"), TqTaskStatus::Completed),
            ]),
            session("parent-session", None, vec![
                task("inherited", 1, "Inherited task", Some("2026-01-02T00:00:00Z"), TqTaskStatus::Todo),
            ]),
        ],
        HashMap::from([("child-session".to_string(), vec![
            view_task("session-specific", 2, "Session task", true),
            view_task("inherited", 1, "Inherited task", false),
        ])]),
    )]
    #[case::preserves_tq_order_for_missing_timestamps(
        vec![session("session-a", None, vec![
            task("first", 1, "First task", None, TqTaskStatus::Todo),
            task("second", 2, "Second task", None, TqTaskStatus::Todo),
        ])],
        HashMap::from([("session-a".to_string(), vec![
            view_task("first", 1, "First task", false),
            view_task("second", 2, "Second task", false),
        ])]),
    )]
    #[case::preserves_tq_order_for_equal_timestamps(
        vec![session("session-a", None, vec![
            task("first", 1, "First task", Some("2026-01-01T00:00:00Z"), TqTaskStatus::Todo),
            task("second", 2, "Second task", Some("2026-01-01T00:00:00Z"), TqTaskStatus::Todo),
        ])],
        HashMap::from([("session-a".to_string(), vec![
            view_task("first", 1, "First task", false),
            view_task("second", 2, "Second task", false),
        ])]),
    )]
    #[case::preserves_parent_task_id(
        vec![session("session-a", None, vec![
            task_with_parent("child-task", 1, "Child task", "parent-task", TqTaskStatus::Todo),
        ])],
        HashMap::from([("session-a".to_string(), vec![SessionTask {
            parent_task_id: Some("parent-task".to_string()),
            ..view_task("child-task", 1, "Child task", false)
        }])]),
    )]
    #[case::drops_unrequested_sessions(
        vec![
            session("session-a", None, vec![task("requested", 1, "Requested", None, TqTaskStatus::Todo)]),
            session("session-b", None, vec![task("unrequested", 2, "Unrequested", None, TqTaskStatus::Todo)]),
        ],
        HashMap::from([("session-a".to_string(), vec![view_task("requested", 1, "Requested", false)])]),
    )]
    fn builds_all_tasks_by_session(
        #[case] sessions: Vec<SessionTasks>,
        #[case] expected: HashMap<String, Vec<SessionTask>>,
    ) {
        assert_eq!(
            build_all_tasks_by_session(
                sessions,
                &HashSet::from(["session-a".to_string(), "child-session".to_string()]),
            ),
            expected,
        );
    }
}
