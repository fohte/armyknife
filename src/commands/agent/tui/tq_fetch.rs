//! Fetches which locally known Claude Code sessions are linked to a tq
//! task, keyed by session_id, for the session list's task-number column.
//! Pure read-only against tq; never touches local session state.

use super::tq_snapshot::TqSnapshot;
use crate::commands::agent::tq_session_tasks::{
    build_all_tasks_by_session, fetch_session_tasks_with_parents, order_tasks_by_priority,
    task_priority_metadata,
};
use crate::infra::tq::{TqClient, TqTaskSummary};
use std::collections::{HashMap, HashSet};
use std::future::Future;

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
    let mut session_tasks = build_all_tasks_by_session(sessions, &local_session_ids);
    if let Err(error) = order_session_tasks_by_priority(&mut session_tasks, |task_ids| async move {
        TqClient
            .list_tasks(&task_ids)
            .await
            .map_err(|error| error.to_string())
    })
    .await
    {
        tracing::warn!(%error, "Failed to fetch tq task priorities for agent sessions");
    }

    Ok(Some(TqSnapshot::new(session_tasks)))
}

async fn order_session_tasks_by_priority<F, Fut>(
    session_tasks: &mut HashMap<String, Vec<super::session_rows::SessionTask>>,
    fetch_task_summaries: F,
) -> Result<(), String>
where
    F: FnOnce(HashSet<String>) -> Fut,
    Fut: Future<Output = Result<Vec<TqTaskSummary>, String>>,
{
    let task_ids = session_tasks
        .values()
        .flatten()
        .map(|task| task.task_id.clone())
        .collect::<HashSet<_>>();
    if task_ids.is_empty() {
        return Ok(());
    }

    let tasks = fetch_task_summaries(task_ids).await?;
    let (_, priority_order) = task_priority_metadata(tasks);
    order_tasks_by_priority(session_tasks, &priority_order);

    Ok(())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::commands::agent::tq_session_tasks::SessionTask;
    use crate::commands::agent::tq_session_tasks::fetch_missing_parent_sessions;
    use crate::infra::tq::{SessionTasks, TqTask, TqTaskSummary};

    fn ids(values: &[&str]) -> HashSet<String> {
        values.iter().map(|s| s.to_string()).collect()
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

    fn linked_task(task_id: &str) -> SessionTask {
        SessionTask {
            task_id: task_id.to_string(),
            task_number: 1,
            task_title: task_id.to_string(),
            parent_task_id: None,
            is_closed: false,
        }
    }

    fn task_summary(task_id: &str, priority: Option<i32>) -> TqTaskSummary {
        TqTaskSummary {
            id: task_id.to_string(),
            number: 1,
            title: task_id.to_string(),
            priority,
            project_id: None,
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

    #[tokio::test]
    async fn priority_lookup_orders_tasks_using_the_injected_task_list() {
        let mut session_tasks = HashMap::from([(
            "session-a".to_string(),
            vec![linked_task("lower"), linked_task("higher")],
        )]);
        let mut requested_ids = None;

        let result = order_session_tasks_by_priority(&mut session_tasks, |task_ids| {
            requested_ids = Some(task_ids);
            async {
                Ok(vec![
                    task_summary("higher", Some(0)),
                    task_summary("lower", Some(1)),
                ])
            }
        })
        .await;

        assert_eq!(
            (requested_ids, result, session_tasks),
            (
                Some(ids(&["lower", "higher"])),
                Ok(()),
                HashMap::from([(
                    "session-a".to_string(),
                    vec![linked_task("higher"), linked_task("lower")],
                )]),
            ),
        );
    }

    #[tokio::test]
    async fn failed_priority_lookup_preserves_the_existing_task_order() {
        let mut session_tasks = HashMap::from([(
            "session-a".to_string(),
            vec![linked_task("first"), linked_task("second")],
        )]);

        let result = order_session_tasks_by_priority(&mut session_tasks, |_| async {
            Err::<Vec<TqTaskSummary>, _>("task list unavailable".to_string())
        })
        .await;

        assert_eq!(
            (result, session_tasks),
            (
                Err("task list unavailable".to_string()),
                HashMap::from([(
                    "session-a".to_string(),
                    vec![linked_task("first"), linked_task("second")],
                )]),
            ),
        );
    }
}
