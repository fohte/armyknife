use std::collections::{HashMap, HashSet};

use crate::commands::agent::tq_session_tasks::order_tasks_by_priority;
use crate::infra::github::PrInfo;

use super::{BranchPrKey, SessionTaskMetadata};

pub(super) fn merge_pull_request_results(
    cached: &mut HashMap<BranchPrKey, Option<PrInfo>>,
    refreshed: HashMap<BranchPrKey, Option<PrInfo>>,
    requested_keys: &HashSet<BranchPrKey>,
) {
    for key in requested_keys {
        if let Some(pull_request) = refreshed.get(key) {
            cached.insert(key.clone(), pull_request.clone());
        }
    }
}

pub(super) fn prune_pull_request_results(
    cached: &mut HashMap<BranchPrKey, Option<PrInfo>>,
    stale_keys: &HashSet<BranchPrKey>,
) {
    cached.retain(|key, _| !stale_keys.contains(key));
}

pub(super) fn merge_session_task_metadata(
    cached: &mut SessionTaskMetadata,
    mut refreshed: SessionTaskMetadata,
    requested_session_ids: &HashSet<String>,
) {
    let previous_task_ids = replace_session_links(cached, &mut refreshed, requested_session_ids);
    prune_unlinked_task_metadata(cached, &previous_task_ids);

    let refreshed_task_ids = cached
        .links
        .iter()
        .filter(|(session_id, _)| requested_session_ids.contains(*session_id))
        .flat_map(|(_, tasks)| tasks.iter().map(|task| task.task_id.clone()))
        .collect::<HashSet<_>>();
    merge_refreshed_task_metadata(cached, &refreshed, &refreshed_task_ids);
    order_tasks_by_priority(&mut cached.links, &cached.priority_order);
}

fn replace_session_links(
    cached: &mut SessionTaskMetadata,
    refreshed: &mut SessionTaskMetadata,
    requested_session_ids: &HashSet<String>,
) -> HashSet<String> {
    let previous_task_ids = cached
        .links
        .iter()
        .filter(|(session_id, _)| requested_session_ids.contains(*session_id))
        .flat_map(|(_, tasks)| tasks.iter().map(|task| task.task_id.clone()))
        .collect::<HashSet<_>>();

    for session_id in requested_session_ids {
        cached.links.remove(session_id);
        if let Some(tasks) = refreshed.links.remove(session_id) {
            cached.links.insert(session_id.clone(), tasks);
        }
    }

    previous_task_ids
}

fn prune_unlinked_task_metadata(
    cached: &mut SessionTaskMetadata,
    previous_task_ids: &HashSet<String>,
) {
    let remaining_task_ids = cached
        .links
        .values()
        .flatten()
        .map(|task| task.task_id.clone())
        .collect::<HashSet<_>>();
    for task_id in previous_task_ids.difference(&remaining_task_ids) {
        cached.priorities.remove(task_id);
        cached.priority_order.remove(task_id);
        cached.task_projects.remove(task_id);
    }
}

fn merge_refreshed_task_metadata(
    cached: &mut SessionTaskMetadata,
    refreshed: &SessionTaskMetadata,
    refreshed_task_ids: &HashSet<String>,
) {
    let mut order_by_priority = cached
        .priorities
        .iter()
        .filter_map(|(task_id, priority)| {
            Some(((*priority)?, *cached.priority_order.get(task_id)?))
        })
        .collect::<HashMap<_, _>>();
    let mut next_priority_order = cached
        .priority_order
        .values()
        .copied()
        .max()
        .map_or(0, |order| order + 1);

    for task_id in refreshed_task_ids {
        cached.priority_order.remove(task_id);
        if let Some(priority) = refreshed.priorities.get(task_id) {
            cached.priorities.insert(task_id.clone(), *priority);
        } else {
            cached.priorities.remove(task_id);
        }
        if let Some(project) = refreshed.task_projects.get(task_id) {
            cached
                .task_projects
                .insert(task_id.clone(), project.clone());
        } else {
            cached.task_projects.remove(task_id);
        }
    }

    let mut refreshed_order = refreshed
        .priority_order
        .iter()
        .filter(|(task_id, _)| refreshed_task_ids.contains(*task_id))
        .collect::<Vec<_>>();
    refreshed_order.sort_by_key(|(task_id, order)| (**order, (*task_id).clone()));
    for (task_id, _) in refreshed_order {
        let Some(Some(priority)) = refreshed.priorities.get(task_id) else {
            continue;
        };
        let order = *order_by_priority.entry(*priority).or_insert_with(|| {
            let order = next_priority_order;
            next_priority_order += 1;
            order
        });
        cached.priority_order.insert(task_id.clone(), order);
    }
}

pub(super) fn prune_session_task_metadata(
    cached: &mut SessionTaskMetadata,
    stale_session_ids: &HashSet<String>,
) {
    let stale_task_ids = cached
        .links
        .iter()
        .filter(|(session_id, _)| stale_session_ids.contains(*session_id))
        .flat_map(|(_, tasks)| tasks.iter().map(|task| task.task_id.clone()))
        .collect::<HashSet<_>>();
    cached
        .links
        .retain(|session_id, _| !stale_session_ids.contains(session_id));
    prune_unlinked_task_metadata(cached, &stale_task_ids);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::github::PrState;
    use rstest::{fixture, rstest};
    use std::sync::Arc;
    use tokio::sync::watch;

    use super::super::super::EnrichmentUpdates;
    use super::super::test_support::{
        branch_key, cache_with_fetchers, linked_task, metadata, session, session_with_branch_name,
        wait_for_refresh,
    };
    use super::super::{
        FETCH_TIMEOUT, PullRequestFetcher, SessionTaskMetadata, TaskMetadataFetcher,
    };

    #[fixture]
    fn updates_and_receiver() -> (EnrichmentUpdates, watch::Receiver<()>) {
        let updates = EnrichmentUpdates::new();
        (updates.clone(), updates.subscribe())
    }

    fn metadata_with_task_details(session_ids: HashSet<String>) -> SessionTaskMetadata {
        let mut result = metadata(session_ids.clone());
        for session_id in session_ids {
            let task_id = format!("task-{session_id}");
            let priority = if session_id == "session-a" { 0 } else { 1 };
            result.priorities.insert(task_id.clone(), Some(priority));
            result
                .priority_order
                .insert(task_id.clone(), priority as usize);
            result
                .task_projects
                .insert(task_id, format!("project-{session_id}"));
        }
        result
    }

    #[rstest]
    #[tokio::test]
    async fn alternating_session_sets_keep_each_sessions_task_metadata(
        updates_and_receiver: (EnrichmentUpdates, watch::Receiver<()>),
    ) {
        let (updates, mut receiver) = updates_and_receiver;
        let task_fetcher: TaskMetadataFetcher = Arc::new(|session_ids, _| {
            Box::pin(async move { Ok(metadata_with_task_details(session_ids)) })
        });
        let pull_request_fetcher: PullRequestFetcher =
            Arc::new(|_| Box::pin(async { Ok(HashMap::new()) }));
        let cache = cache_with_fetchers(updates, FETCH_TIMEOUT, task_fetcher, pull_request_fetcher);
        let session_a = session("session-a", None);
        let session_b = session("session-b", None);

        let initial = cache.task_metadata(std::slice::from_ref(&session_a)).await;
        wait_for_refresh(&mut receiver).await;
        let cached_a = cache.task_metadata(std::slice::from_ref(&session_a)).await;
        let cached_while_b_refreshes = cache.task_metadata(std::slice::from_ref(&session_b)).await;
        wait_for_refresh(&mut receiver).await;
        let cached_a_after_b_refresh = cache.task_metadata(std::slice::from_ref(&session_a)).await;

        assert_eq!(
            (
                initial,
                cached_a,
                cached_while_b_refreshes,
                cached_a_after_b_refresh,
            ),
            (
                SessionTaskMetadata::default(),
                metadata_with_task_details(HashSet::from(["session-a".to_string()])),
                metadata_with_task_details(HashSet::from(["session-a".to_string()])),
                metadata_with_task_details(HashSet::from([
                    "session-a".to_string(),
                    "session-b".to_string(),
                ])),
            )
        );
    }

    #[rstest]
    #[tokio::test]
    async fn alternating_branch_sets_keep_each_cached_pull_request(
        updates_and_receiver: (EnrichmentUpdates, watch::Receiver<()>),
    ) {
        let (updates, mut receiver) = updates_and_receiver;
        let pull_request_fetcher: PullRequestFetcher = Arc::new(|sessions| {
            Box::pin(async move {
                Ok(sessions
                    .iter()
                    .filter_map(|session| {
                        let context = session.pr_context.as_ref()?;
                        Some((
                            (
                                context.owner.clone(),
                                context.repo.clone(),
                                context.branch.clone(),
                            ),
                            Some(PrInfo {
                                number: if context.branch == "branch-a" { 17 } else { 23 },
                                title: format!("Change for {}", context.branch),
                                state: PrState::Open,
                                url: format!("https://example.test/{}", context.branch),
                            }),
                        ))
                    })
                    .collect())
            })
        });
        let task_fetcher: TaskMetadataFetcher =
            Arc::new(|_, previous| Box::pin(async move { Ok(previous) }));
        let cache = cache_with_fetchers(updates, FETCH_TIMEOUT, task_fetcher, pull_request_fetcher);
        let session_a = session_with_branch_name("session-a", "branch-a");
        let session_b = session_with_branch_name("session-b", "branch-b");

        let initial = cache.pull_requests(std::slice::from_ref(&session_a)).await;
        wait_for_refresh(&mut receiver).await;
        let cached_a = cache.pull_requests(std::slice::from_ref(&session_a)).await;
        let cached_while_b_refreshes = cache.pull_requests(std::slice::from_ref(&session_b)).await;
        wait_for_refresh(&mut receiver).await;
        let cached_a_after_b_refresh = cache.pull_requests(std::slice::from_ref(&session_a)).await;
        let cached_b_after_refresh = cache.pull_requests(std::slice::from_ref(&session_b)).await;

        assert_eq!(
            (
                initial,
                cached_a,
                cached_while_b_refreshes,
                cached_a_after_b_refresh,
                cached_b_after_refresh,
            ),
            (
                HashMap::from([("session-a".to_string(), None)]),
                HashMap::from([(
                    "session-a".to_string(),
                    Some(PrInfo {
                        number: 17,
                        title: "Change for branch-a".to_string(),
                        state: PrState::Open,
                        url: "https://example.test/branch-a".to_string(),
                    }),
                )]),
                HashMap::from([("session-b".to_string(), None)]),
                HashMap::from([(
                    "session-a".to_string(),
                    Some(PrInfo {
                        number: 17,
                        title: "Change for branch-a".to_string(),
                        state: PrState::Open,
                        url: "https://example.test/branch-a".to_string(),
                    }),
                )]),
                HashMap::from([(
                    "session-b".to_string(),
                    Some(PrInfo {
                        number: 23,
                        title: "Change for branch-b".to_string(),
                        state: PrState::Open,
                        url: "https://example.test/branch-b".to_string(),
                    }),
                )]),
            )
        );
    }

    #[test]
    fn prunes_stale_session_links_and_unreferenced_task_metadata() {
        let mut cached = metadata(HashSet::from([
            "session-a".to_string(),
            "session-b".to_string(),
        ]));
        cached.priorities = HashMap::from([
            ("task-session-a".to_string(), Some(0)),
            ("task-session-b".to_string(), Some(1)),
        ]);
        cached.priority_order = HashMap::from([
            ("task-session-a".to_string(), 0),
            ("task-session-b".to_string(), 1),
        ]);
        cached.task_projects = HashMap::from([
            ("task-session-a".to_string(), "project-a".to_string()),
            ("task-session-b".to_string(), "project-b".to_string()),
        ]);
        cached
            .links
            .get_mut("session-b")
            .unwrap()
            .push(linked_task("task-session-a"));

        prune_session_task_metadata(&mut cached, &HashSet::from(["session-a".to_string()]));

        assert_eq!(
            cached,
            SessionTaskMetadata {
                links: HashMap::from([(
                    "session-b".to_string(),
                    vec![linked_task("task-session-b"), linked_task("task-session-a"),],
                )]),
                priorities: HashMap::from([
                    ("task-session-a".to_string(), Some(0)),
                    ("task-session-b".to_string(), Some(1)),
                ]),
                priority_order: HashMap::from([
                    ("task-session-a".to_string(), 0),
                    ("task-session-b".to_string(), 1),
                ]),
                task_projects: HashMap::from([
                    ("task-session-a".to_string(), "project-a".to_string()),
                    ("task-session-b".to_string(), "project-b".to_string()),
                ]),
            }
        );
    }

    #[test]
    fn prunes_stale_pull_request_branch_results() {
        let branch_a = branch_key("branch-a");
        let branch_b = branch_key("branch-b");
        let mut cached = HashMap::from([
            (branch_a.clone(), None),
            (
                branch_b.clone(),
                Some(PrInfo {
                    number: 23,
                    title: "Change for branch-b".to_string(),
                    state: PrState::Open,
                    url: "https://example.test/branch-b".to_string(),
                }),
            ),
        ]);

        prune_pull_request_results(&mut cached, &HashSet::from([branch_a.clone()]));

        assert_eq!(
            cached,
            HashMap::from([(
                branch_b,
                Some(PrInfo {
                    number: 23,
                    title: "Change for branch-b".to_string(),
                    state: PrState::Open,
                    url: "https://example.test/branch-b".to_string(),
                })
            )])
        );
    }
}
