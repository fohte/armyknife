//! Fetches which locally known Claude Code sessions are linked to a tq
//! task, keyed by session_id, for the session list's task-number column.
//! Pure read-only against tq; never touches local session state.

use super::tq_snapshot::TqSnapshot;
use crate::commands::agent::tq_session_tasks::{
    build_all_tasks_by_session, fetch_session_tasks_with_parents,
};
use crate::infra::tq::TqClient;
use std::collections::HashSet;

/// Fetches tq task links for local sessions and their missing parent sessions.
///
/// Returns `Ok(None)` when tq is unavailable or there are no local sessions.
/// A successful lookup returns a snapshot even when no tasks are linked, so
/// callers can clear stale associations.
pub async fn fetch_task_snapshot(
    client: Option<TqClient>,
    local_session_ids: HashSet<String>,
) -> Result<Option<TqSnapshot>, String> {
    if local_session_ids.is_empty() {
        return Ok(None);
    }
    let Some(client) = client else {
        return Ok(None);
    };

    let sessions = fetch_session_tasks_with_parents(&client, &local_session_ids).await?;
    let session_tasks = build_all_tasks_by_session(sessions, &local_session_ids);

    Ok(Some(TqSnapshot::new(session_tasks)))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rstest::rstest;

    use super::*;
    use crate::commands::agent::tq_session_tasks::{
        SessionTask, build_task_by_session, fetch_missing_parent_sessions,
    };
    use crate::infra::tq::{SessionTasks, TqTask, TqTaskStatus};

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
    async fn missing_client_skips_without_spawning_tq() {
        let result = fetch_task_snapshot(None, ids(&["session-1"])).await;

        assert_eq!(result, Ok(None));
    }

    #[tokio::test]
    async fn empty_local_session_ids_skip_without_spawning_tq() {
        let result = fetch_task_snapshot(Some(TqClient), HashSet::new()).await;

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
    #[case::missing_timestamps_prefer_open_task_across_statuses(
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
                task_id: "task-2".to_string(),
                task_number: 2,
                task_title: "Open task".to_string(),
                parent_task_id: None,
                is_closed: false,
            },
        )]),
    )]
    #[case::open_task_precedes_later_completed_task(
        vec![session(
            "session-a",
            vec![
                linked_task("task-1", 1, "Open task", None, "2026-01-01T00:00:00Z"),
                closed_linked_task("task-2", 2, "Completed task", None, "2026-01-02T00:00:00Z"),
            ],
        )],
        &["session-a"],
        HashMap::from([(
            "session-a".to_string(),
            SessionTask {
                task_id: "task-1".to_string(),
                task_number: 1,
                task_title: "Open task".to_string(),
                parent_task_id: None,
                is_closed: false,
            },
        )]),
    )]
    #[case::newest_open_task_is_first(
        vec![session(
            "session-a",
            vec![
                linked_task("task-1", 1, "Earlier task", None, "2026-01-01T00:00:00Z"),
                linked_task("task-2", 2, "Later task", None, "2026-01-02T00:00:00Z"),
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
                is_closed: false,
            },
        )]),
    )]
    #[case::newest_completed_task_is_first(
        vec![session(
            "session-a",
            vec![
                closed_linked_task("task-1", 1, "Earlier task", None, "2026-01-01T00:00:00Z"),
                closed_linked_task("task-2", 2, "Later task", None, "2026-01-02T00:00:00Z"),
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
