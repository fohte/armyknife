//! Fetches which locally known Claude Code sessions are linked to a tq
//! task, keyed by session_id, for the session list's task-number column.
//! Pure read-only against tq; never touches local session state.

use std::collections::{HashMap, HashSet};
use std::future::Future;

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

    let linked_task_ids = sessions
        .iter()
        .filter(|session| local_session_ids.contains(&session.session_id))
        .flat_map(|session| session.tasks.iter().map(|task| task.id.clone()))
        .collect::<HashSet<_>>();

    let client_ref = &client;
    let parent_sessions = fetch_missing_parent_sessions(
        &sessions,
        &local_session_ids,
        |parent_session_ids| async move { client_ref.list_session_tasks(&parent_session_ids).await },
    );
    let (mut parent_sessions, tasks, projects) = if linked_task_ids.is_empty() {
        let (parent_sessions, projects) =
            tokio::try_join!(parent_sessions, client.list_projects()).map_err(|e| e.to_string())?;
        (parent_sessions, Vec::new(), projects)
    } else {
        tokio::try_join!(
            parent_sessions,
            client.list_sidebar_tasks(&linked_task_ids),
            client.list_projects(),
        )
        .map_err(|e| e.to_string())?
    };

    let mut sessions = sessions;
    sessions.append(&mut parent_sessions);
    let session_tasks = build_all_tasks_by_session(sessions, &local_session_ids);

    Ok(Some(TqSnapshot::new(session_tasks, tasks, projects)))
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

async fn fetch_missing_parent_sessions<F, Fut, E>(
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

/// Reduces tq's session -> tasks listing to one [`SessionTask`] per locally
/// known session_id. It prefers tasks not linked to the parent session, then
/// newer links, then open tasks at equal timestamps. Other ties and responses
/// without timestamps retain tq's ordering.
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
                let inherited_order =
                    inherited_task_ids.map_or(std::cmp::Ordering::Equal, |task_ids| {
                        task_ids
                            .contains(&left.id)
                            .cmp(&task_ids.contains(&right.id))
                    });
                inherited_order
                    .then_with(|| right.linked_at.cmp(&left.linked_at))
                    .then_with(|| {
                        if left.linked_at.is_some() {
                            (left.status == TqTaskStatus::Completed)
                                .cmp(&(right.status == TqTaskStatus::Completed))
                        } else {
                            std::cmp::Ordering::Equal
                        }
                    })
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
            linked_at: None,
            parent_id: parent_id.map(String::from),
            status: TqTaskStatus::Todo,
        }
    }

    fn linked_task(
        id: &str,
        number: u32,
        title: &str,
        parent_id: Option<&str>,
        linked_at: &str,
    ) -> TqTask {
        TqTask {
            linked_at: Some(
                chrono::DateTime::parse_from_rfc3339(linked_at)
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
            ..task(id, number, title, parent_id)
        }
    }

    fn closed_task(id: &str, number: u32, title: &str, parent_id: Option<&str>) -> TqTask {
        TqTask {
            status: TqTaskStatus::Completed,
            ..task(id, number, title, parent_id)
        }
    }

    fn closed_linked_task(
        id: &str,
        number: u32,
        title: &str,
        parent_id: Option<&str>,
        linked_at: &str,
    ) -> TqTask {
        TqTask {
            status: TqTaskStatus::Completed,
            ..linked_task(id, number, title, parent_id, linked_at)
        }
    }

    fn session(session_id: &str, tasks: Vec<TqTask>) -> SessionTasks {
        SessionTasks {
            session_id: session_id.to_string(),
            parent_session_id: None,
            tasks,
        }
    }

    fn child_session(
        session_id: &str,
        parent_session_id: &str,
        tasks: Vec<TqTask>,
    ) -> SessionTasks {
        SessionTasks {
            parent_session_id: Some(parent_session_id.to_string()),
            ..session(session_id, tasks)
        }
    }

    #[rstest]
    #[case::fetches_missing_parent_and_filters_unrequested_sessions(
        vec![
            child_session("session-a", "parent-session", vec![]),
            child_session("remote-session", "remote-parent", vec![]),
        ],
        &["session-a"],
        vec![
            session("parent-session", vec![]),
            session("remote-parent", vec![]),
        ],
        Some(ids(&["parent-session"])),
        vec![session("parent-session", vec![])],
    )]
    #[case::skips_query_when_parent_is_already_present(
        vec![
            child_session("session-a", "parent-session", vec![]),
            session("parent-session", vec![]),
        ],
        &["session-a"],
        vec![],
        None,
        vec![],
    )]
    #[tokio::test]
    async fn fetch_missing_parent_sessions_cases(
        #[case] sessions: Vec<SessionTasks>,
        #[case] local_session_ids: &[&str],
        #[case] response: Vec<SessionTasks>,
        #[case] expected_requested_parent_session_ids: Option<HashSet<String>>,
        #[case] expected_parent_sessions: Vec<SessionTasks>,
    ) {
        let mut requested_parent_session_ids = None;
        let parent_sessions = fetch_missing_parent_sessions(
            &sessions,
            &ids(local_session_ids),
            |parent_session_ids| {
                requested_parent_session_ids = Some(parent_session_ids);
                async move { Ok::<_, ()>(response) }
            },
        )
        .await;

        assert_eq!(
            (requested_parent_session_ids, parent_sessions,),
            (
                expected_requested_parent_session_ids,
                Ok(expected_parent_sessions),
            ),
        );
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
    #[case::missing_timestamps_preserve_tq_order(
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
    #[case::missing_timestamps_preserve_tq_order_across_statuses(
        vec![session(
            "session-a",
            vec![
                closed_task("task-1", 1, "Completed task", None),
                task("task-2", 2, "Open task", None),
            ],
        )],
        &["session-a"],
        HashMap::from([(
            "session-a".to_string(),
            SessionTask {
                task_id: "task-1".to_string(),
                task_number: 1,
                task_title: "Completed task".to_string(),
                parent_task_id: None,
                is_closed: true,
            },
        )]),
    )]
    #[case::newest_linked_task_is_first(
        vec![session(
            "session-a",
            vec![
                linked_task("task-1", 1, "Earlier task", None, "2026-01-02T00:00:00+09:00"),
                closed_linked_task("task-2", 2, "Later task", None, "2026-01-01T16:00:00Z"),
            ],
        )],
        &["session-a"],
        HashMap::from([(
            "session-a".to_string(),
            SessionTask {
                task_id: "task-2".to_string(),
                task_number: 2,
                task_title: "Later task".to_string(),
                parent_task_id: None,
                is_closed: true,
            },
        )]),
    )]
    #[case::equal_timestamps_prefer_open_task(
        vec![session(
            "session-a",
            vec![
                closed_linked_task("task-1", 1, "Completed task", None, "2026-01-01T00:00:00Z"),
                linked_task("task-2", 2, "Open task", None, "2026-01-01T00:00:00Z"),
            ],
        )],
        &["session-a"],
        HashMap::from([(
            "session-a".to_string(),
            SessionTask {
                task_id: "task-2".to_string(),
                task_number: 2,
                task_title: "Open task".to_string(),
                parent_task_id: None,
                is_closed: false,
            },
        )]),
    )]
    #[case::task_not_linked_to_parent_precedes_inherited_task(
        vec![
            child_session(
                "session-a",
                "parent-session",
                vec![
                    linked_task("task-1", 1, "Inherited task", None, "2026-01-02T00:00:00Z"),
                    closed_linked_task("task-2", 2, "Session task", None, "2026-01-01T00:00:00Z"),
                ],
            ),
            session(
                "parent-session",
                vec![linked_task("task-1", 1, "Inherited task", None, "2026-01-02T00:00:00Z")],
            ),
        ],
        &["session-a"],
        HashMap::from([(
            "session-a".to_string(),
            SessionTask {
                task_id: "task-2".to_string(),
                task_number: 2,
                task_title: "Session task".to_string(),
                parent_task_id: None,
                is_closed: true,
            },
        )]),
    )]
    #[case::equal_timestamps_preserve_tq_order(
        vec![session(
            "session-a",
            vec![
                linked_task("task-1", 1, "First task", None, "2026-01-01T00:00:00Z"),
                linked_task("task-2", 2, "Second task", None, "2026-01-01T00:00:00Z"),
            ],
        )],
        &["session-a"],
        HashMap::from([(
            "session-a".to_string(),
            SessionTask {
                task_id: "task-1".to_string(),
                task_number: 1,
                task_title: "First task".to_string(),
                parent_task_id: None,
                is_closed: false,
            },
        )]),
    )]
    #[case::equal_completed_timestamps_preserve_tq_order(
        vec![session(
            "session-a",
            vec![
                closed_linked_task("task-1", 1, "First task", None, "2026-01-01T00:00:00Z"),
                closed_linked_task("task-2", 2, "Second task", None, "2026-01-01T00:00:00Z"),
            ],
        )],
        &["session-a"],
        HashMap::from([(
            "session-a".to_string(),
            SessionTask {
                task_id: "task-1".to_string(),
                task_number: 1,
                task_title: "First task".to_string(),
                parent_task_id: None,
                is_closed: true,
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
