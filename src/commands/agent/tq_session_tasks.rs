use std::collections::{HashMap, HashSet};
use std::future::Future;

use crate::commands::agent::claude_sessions::normalize_title;
use crate::infra::tq::{SessionTasks, TqClient, TqTask, TqTaskStatus, TqTaskSummary};

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

pub(super) fn task_priority_metadata(
    tasks: Vec<TqTaskSummary>,
) -> (HashMap<String, Option<i32>>, HashMap<String, usize>) {
    let mut order_by_priority = HashMap::new();
    let mut priorities = HashMap::new();
    let mut priority_order = HashMap::new();
    for task in tasks {
        let order = task.priority.map(|priority| {
            let next_order = order_by_priority.len();
            *order_by_priority.entry(priority).or_insert(next_order)
        });
        priorities.insert(task.id.clone(), task.priority);
        if let Some(order) = order {
            priority_order.insert(task.id, order);
        }
    }

    (priorities, priority_order)
}

pub(super) fn order_tasks_by_priority(
    tasks_by_session: &mut HashMap<String, Vec<SessionTask>>,
    priority_order: &HashMap<String, usize>,
) {
    for tasks in tasks_by_session.values_mut() {
        tasks.sort_by(|left, right| {
            left.is_closed.cmp(&right.is_closed).then_with(|| {
                if left.is_closed {
                    std::cmp::Ordering::Equal
                } else {
                    priority_order
                        .get(&left.task_id)
                        .unwrap_or(&usize::MAX)
                        .cmp(priority_order.get(&right.task_id).unwrap_or(&usize::MAX))
                }
            })
        });
    }
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
    use super::order_tasks_by_priority;
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

    #[rstest]
    #[case::open_priority_precedes_link_order_and_unset_is_lowest(
        vec![
            view_task("recent", 1, "Recent", false),
            view_task("high", 2, "High priority", false),
            view_task("unset", 3, "Unset priority", false),
            view_task("completed", 4, "Completed", true),
        ],
        HashMap::from([("high".to_string(), 0), ("recent".to_string(), 1)]),
        vec!["high", "recent", "unset", "completed"],
    )]
    #[case::equal_priorities_preserve_existing_order(
        vec![
            view_task("first", 1, "First", false),
            view_task("second", 2, "Second", false),
        ],
        HashMap::from([("first".to_string(), 0), ("second".to_string(), 0)]),
        vec!["first", "second"],
    )]
    #[case::completed_tasks_keep_existing_order(
        vec![
            view_task("first", 1, "First", true),
            view_task("second", 2, "Second", true),
        ],
        HashMap::from([("second".to_string(), 0)]),
        vec!["first", "second"],
    )]
    fn orders_linked_tasks_by_open_priority_then_existing_order(
        #[case] tasks: Vec<SessionTask>,
        #[case] priority_order: HashMap<String, usize>,
        #[case] expected_ids: Vec<&str>,
    ) {
        let mut tasks_by_session = HashMap::from([("session-a".to_string(), tasks)]);

        order_tasks_by_priority(&mut tasks_by_session, &priority_order);

        assert_eq!(
            tasks_by_session["session-a"]
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>(),
            expected_ids,
        );
    }
}
