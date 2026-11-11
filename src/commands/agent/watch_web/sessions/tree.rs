use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Serialize;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct TreeSessionView {
    id: String,
    parent_id: Option<String>,
    category: SessionCategory,
    is_active: bool,
    kind: Option<SessionKind>,
    title: String,
    preview: Option<String>,
    elapsed_seconds: u64,
    task: Option<TqTaskView>,
    repo: String,
    decision_count: i64,
    pull_request: Option<PullRequestView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionTreeResponse {
    revision: i64,
    sessions: Vec<TreeSessionView>,
}

fn session_parent_ids(
    tree_sessions: &[Session],
    tree_session_ids: &HashSet<&str>,
) -> HashMap<String, Option<String>> {
    let mut parent_ids = tree_sessions
        .iter()
        .map(|session| {
            let parent_id = session
                .ancestor_session_ids
                .iter()
                .rev()
                .find(|ancestor_id| tree_session_ids.contains(ancestor_id.as_str()))
                .cloned();
            (session.session_id.clone(), parent_id)
        })
        .collect::<HashMap<_, _>>();
    remove_parent_cycles(&mut parent_ids);
    parent_ids
}

fn remove_parent_cycles(parent_ids: &mut HashMap<String, Option<String>>) {
    let mut completed = HashSet::new();
    let mut starts = parent_ids.keys().cloned().collect::<Vec<_>>();
    starts.sort();
    for start in starts {
        if completed.contains(&start) {
            continue;
        }
        let mut path = Vec::<String>::new();
        let mut path_positions = HashMap::<String, usize>::new();
        let mut current_id = start;
        loop {
            if completed.contains(&current_id) {
                break;
            }
            if let Some(cycle_start) = path_positions.get(&current_id).copied() {
                // A stable root keeps inconsistent ancestry renderable.
                if let Some(root_id) = path[cycle_start..].iter().min() {
                    parent_ids.insert(root_id.clone(), None);
                }
                break;
            }
            path_positions.insert(current_id.clone(), path.len());
            path.push(current_id.clone());
            let Some(parent_id) = parent_ids.get(&current_id).cloned().flatten() else {
                break;
            };
            current_id = parent_id;
        }
        completed.extend(path);
    }
}

pub(super) async fn session_tree<S: SessionSource>(
    Path(session_id): Path<String>,
    State(source): State<S>,
) -> Result<Json<SessionTreeResponse>, StatusCode> {
    let snapshot = match source.load_snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::warn!(%error, "failed to load agent session tree");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let Some(selected_session) = snapshot
        .all_sessions
        .iter()
        .find(|session| session.session_id == session_id)
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    let available_ids = snapshot
        .all_sessions
        .iter()
        .map(|session| session.session_id.as_str())
        .collect::<HashSet<_>>();
    let root_id = selected_session
        .ancestor_session_ids
        .iter()
        .find(|id| available_ids.contains(id.as_str()))
        .cloned()
        .unwrap_or_else(|| selected_session.session_id.clone());
    let tree_sessions = snapshot
        .all_sessions
        .iter()
        .filter(|session| {
            session.session_id == root_id
                || session
                    .ancestor_session_ids
                    .iter()
                    .any(|ancestor_id| ancestor_id == &root_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    let tree_session_ids = tree_sessions
        .iter()
        .map(|session| session.session_id.as_str())
        .collect::<HashSet<_>>();
    let parent_ids = session_parent_ids(&tree_sessions, &tree_session_ids);
    let sessions_to_check = tree_sessions
        .iter()
        .filter(|session| {
            session.engine == Engine::Claude && session.status == SessionStatus::Running
        })
        .cloned()
        .collect::<Vec<_>>();
    let sessions_to_resolve = tree_sessions
        .iter()
        .filter(|session| !snapshot.repo_by_id.contains_key(&session.session_id))
        .cloned()
        .collect::<Vec<_>>();
    let missing_repo_ids = sessions_to_resolve
        .iter()
        .map(|session| session.session_id.clone())
        .collect::<HashSet<_>>();
    let all_sessions_for_repos = snapshot.all_sessions.clone();
    let session_metadata = tokio::task::spawn_blocking(move || {
        (
            interrupted_session_ids(sessions_to_check),
            resolve_repos_for_sessions(&sessions_to_resolve, &all_sessions_for_repos, resolve_repo),
        )
    });
    let (task_metadata, pull_requests) = tokio::join!(
        source.fetch_task_metadata(&tree_sessions),
        source.fetch_pull_requests(&tree_sessions),
    );
    let (interrupted_session_ids, missing_repo_by_id) = session_metadata
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut repo_by_id = snapshot.repo_by_id.clone();
    repo_by_id.extend(
        missing_repo_by_id
            .into_iter()
            .filter(|(session_id, _)| missing_repo_ids.contains(session_id)),
    );
    let turns = user_turns(tree_sessions.iter(), &snapshot.agent_config)
        .into_iter()
        .map(|turn| (turn.session.session_id.as_str(), turn.kind))
        .collect::<HashMap<_, _>>();
    let state_by_id = snapshot
        .web_states
        .iter()
        .map(|state| (state.session_id.as_str(), state))
        .collect::<HashMap<_, _>>();
    let now = source.now();
    let sessions = tree_sessions
        .iter()
        .map(|session| {
            let task = task_metadata
                .links
                .get(&session.session_id)
                .and_then(|tasks| tasks.first())
                .map(|task| TqTaskView {
                    id: task.task_id.clone(),
                    number: task.task_number,
                    title: task.task_title.clone(),
                    priority: task_metadata
                        .priorities
                        .get(&task.task_id)
                        .copied()
                        .flatten(),
                    is_closed: task.is_closed,
                    priority_order: task_metadata.priority_order.get(&task.task_id).copied(),
                });
            let turn = turns.get(session.session_id.as_str()).copied();
            let needs_action = interrupted_session_ids.contains(&session.session_id)
                || snapshot
                    .unanswered_proposal_titles
                    .contains_key(&session.session_id);
            let category = category_for_with_later(
                session,
                turn,
                needs_action,
                state_by_id.get(session.session_id.as_str()).copied(),
                now,
            );
            TreeSessionView {
                id: session.session_id.clone(),
                parent_id: parent_ids.get(&session.session_id).cloned().flatten(),
                category,
                is_active: session.status != SessionStatus::Ended,
                kind: kind_for(
                    session,
                    turn,
                    snapshot
                        .unanswered_proposal_titles
                        .contains_key(&session.session_id),
                ),
                title: snapshot
                    .title_by_id
                    .get(&session.session_id)
                    .cloned()
                    .unwrap_or_default(),
                preview: snapshot
                    .unanswered_proposal_titles
                    .get(&session.session_id)
                    .cloned()
                    .or_else(|| session.last_message.clone()),
                elapsed_seconds: now
                    .signed_duration_since(session.updated_at)
                    .num_seconds()
                    .max(0) as u64,
                task,
                repo: repo_by_id
                    .get(&session.session_id)
                    .cloned()
                    .unwrap_or_default(),
                decision_count: snapshot
                    .decision_counts
                    .get(&session.session_id)
                    .map_or(0, |counts| counts.total),
                pull_request: pull_requests
                    .get(&session.session_id)
                    .and_then(Option::as_ref)
                    .map(|info| PullRequestView {
                        number: info.number,
                        state: info.state.clone(),
                    }),
            }
        })
        .collect();
    Ok(Json(SessionTreeResponse {
        revision: snapshot.revision,
        sessions,
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::remove_parent_cycles;

    #[test]
    fn session_tree_parent_cycles_fall_back_to_the_smallest_session_id() {
        let mut parent_ids = HashMap::from([
            ("cycle-a".to_string(), Some("cycle-b".to_string())),
            ("cycle-b".to_string(), Some("cycle-a".to_string())),
            ("cycle-child".to_string(), Some("cycle-a".to_string())),
            ("self-cycle".to_string(), Some("self-cycle".to_string())),
        ]);

        remove_parent_cycles(&mut parent_ids);

        assert_eq!(
            parent_ids,
            HashMap::from([
                ("cycle-a".to_string(), None),
                ("cycle-b".to_string(), Some("cycle-a".to_string())),
                ("cycle-child".to_string(), Some("cycle-a".to_string())),
                ("self-cycle".to_string(), None),
            ]),
        );
    }
}
