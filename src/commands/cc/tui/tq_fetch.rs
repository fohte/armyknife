//! Fetches which locally known Claude Code sessions are linked to tq tasks,
//! and groups them into [`TaskGroup`]s for the session list to render as a
//! tree. Pure read-only against tq; never touches local session state.

use std::collections::{HashMap, HashSet};

use super::session_rows::TaskGroup;
use crate::infra::tq::TqClient;

/// Fetches tq task links for `local_session_ids` and groups them into one
/// [`TaskGroup`] per distinct task, ordered by the by-task API's own
/// most-recently-active-first order (session_id -> task_id links outside
/// `local_session_ids`, e.g. sessions running on another machine, are
/// dropped -- only sessions this TUI can display or act on are relevant).
///
/// Returns `Ok(vec![])`, not an error, when tq integration isn't configured
/// (`client` is `None`, i.e. `TqClient::from_env()` returned `None`) -- the
/// caller treats "not configured" and "fetched, nothing linked" identically:
/// render the flat status-sectioned list.
pub async fn fetch_task_groups(
    client: Option<TqClient>,
    local_session_ids: HashSet<String>,
) -> Result<Vec<TaskGroup>, String> {
    let Some(client) = client else {
        return Ok(Vec::new());
    };

    let links = client
        .list_task_session_links()
        .await
        .map_err(|e| e.to_string())?;

    // task_id -> member session_ids, insertion-ordered by first appearance
    // (the API sorts by lastActiveAt desc, so this preserves that order).
    let mut order: Vec<String> = Vec::new();
    let mut members: HashMap<String, HashSet<String>> = HashMap::new();
    for link in links {
        if !local_session_ids.contains(&link.session_id) {
            continue;
        }
        if !members.contains_key(&link.task_id) {
            order.push(link.task_id.clone());
        }
        members
            .entry(link.task_id)
            .or_default()
            .insert(link.session_id);
    }

    let mut groups = Vec::with_capacity(order.len());
    for task_id in order {
        // One task's detail fetch failing shouldn't drop every other group.
        let Ok(task) = client.get_task(&task_id).await else {
            continue;
        };
        let session_ids = members.remove(&task_id).unwrap_or_default();
        groups.push(TaskGroup {
            task_number: task.number,
            task_title: task.title,
            session_ids,
        });
    }

    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::tq::mock::TqMockServer;

    fn ids(values: &[&str]) -> HashSet<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn client_none_returns_empty_without_network() {
        let result = fetch_task_groups(None, ids(&["session-1"])).await;

        assert_eq!(result, Ok(Vec::new()));
    }

    #[tokio::test]
    async fn groups_local_sessions_by_task_in_first_seen_order() {
        let mock = TqMockServer::start().await;
        mock.task_session_links(&[
            ("task-2", "session-a"),
            ("task-1", "session-b"),
            ("task-2", "session-c"),
        ])
        .await;
        mock.task("task-2", 20, "Second task").await;
        mock.task("task-1", 10, "First task").await;

        let result = fetch_task_groups(
            Some(mock.client()),
            ids(&["session-a", "session-b", "session-c"]),
        )
        .await;

        assert_eq!(
            result,
            Ok(vec![
                TaskGroup {
                    task_number: 20,
                    task_title: "Second task".to_string(),
                    session_ids: ids(&["session-a", "session-c"]),
                },
                TaskGroup {
                    task_number: 10,
                    task_title: "First task".to_string(),
                    session_ids: ids(&["session-b"]),
                },
            ])
        );
    }

    #[tokio::test]
    async fn drops_links_for_sessions_outside_the_local_set() {
        let mock = TqMockServer::start().await;
        mock.task_session_links(&[("task-1", "session-a"), ("task-1", "remote-session")])
            .await;
        mock.task("task-1", 1, "Task").await;

        let result = fetch_task_groups(Some(mock.client()), ids(&["session-a"])).await;

        assert_eq!(
            result,
            Ok(vec![TaskGroup {
                task_number: 1,
                task_title: "Task".to_string(),
                session_ids: ids(&["session-a"]),
            }])
        );
    }

    #[tokio::test]
    async fn session_linked_to_multiple_tasks_appears_in_each_group() {
        let mock = TqMockServer::start().await;
        mock.task_session_links(&[("task-1", "session-a"), ("task-2", "session-a")])
            .await;
        mock.task("task-1", 1, "Task one").await;
        mock.task("task-2", 2, "Task two").await;

        let result = fetch_task_groups(Some(mock.client()), ids(&["session-a"])).await;

        assert_eq!(
            result,
            Ok(vec![
                TaskGroup {
                    task_number: 1,
                    task_title: "Task one".to_string(),
                    session_ids: ids(&["session-a"]),
                },
                TaskGroup {
                    task_number: 2,
                    task_title: "Task two".to_string(),
                    session_ids: ids(&["session-a"]),
                },
            ])
        );
    }

    #[tokio::test]
    async fn skips_task_whose_detail_fetch_fails_without_dropping_others() {
        let mock = TqMockServer::start().await;
        mock.task_session_links(&[("task-1", "session-a"), ("task-2", "session-b")])
            .await;
        mock.task_error("task-1", 404).await;
        mock.task("task-2", 2, "Task two").await;

        let result = fetch_task_groups(Some(mock.client()), ids(&["session-a", "session-b"])).await;

        assert_eq!(
            result,
            Ok(vec![TaskGroup {
                task_number: 2,
                task_title: "Task two".to_string(),
                session_ids: ids(&["session-b"]),
            }])
        );
    }

    #[tokio::test]
    async fn list_links_error_propagates() {
        let mock = TqMockServer::start().await;
        mock.task_session_links_error(500).await;

        let result = fetch_task_groups(Some(mock.client()), ids(&["session-a"])).await;

        assert_eq!(result, Err("tq API error: HTTP 500".to_string()));
    }
}
