use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::commands::agent::session_title::title_display_name;
use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session, SessionStatus};
use crate::commands::agent::user_turn::{UserTurnKind, user_turns};
use crate::infra::git::{github_owner_and_repo, open_repo_at};
use crate::infra::github::{BranchPrQuery, GitHubClient, PrInfo, PrState};
use crate::infra::tq::{TqClient, TqTask, TqTaskStatus, TqTaskSummary};
use crate::shared::config::{self, AgentConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum SessionCategory {
    NeedsAction,
    InProgress,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
enum SessionKind {
    Question,
    Approve,
    Review,
    Error,
    Plan,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct TqTaskView {
    id: String,
    number: u32,
    title: String,
    priority: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ParentSessionView {
    id: String,
    title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestView {
    number: u64,
    state: PrState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionView {
    id: String,
    engine: Engine,
    category: SessionCategory,
    kind: Option<SessionKind>,
    title: String,
    preview: Option<String>,
    updated_at: DateTime<Utc>,
    elapsed_seconds: u64,
    unread: bool,
    task: Option<TqTaskView>,
    repo: String,
    parent: Option<ParentSessionView>,
    pull_request: Option<PullRequestView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionsResponse {
    revision: i64,
    sessions: Vec<SessionView>,
}

struct SessionSnapshot {
    revision: i64,
    sessions: Vec<Session>,
    title_by_id: HashMap<String, String>,
    repo_by_id: HashMap<String, String>,
    agent_config: AgentConfig,
}

struct TaskLink {
    id: String,
    number: u32,
    title: String,
}

pub(super) fn router() -> Router {
    Router::new().route("/sessions", get(list_sessions))
}

async fn list_sessions() -> Result<Json<SessionsResponse>, StatusCode> {
    let snapshot = tokio::task::spawn_blocking(load_snapshot)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let task_links = fetch_task_links(&snapshot.sessions).await;
    let task_priorities = fetch_task_priorities(&task_links).await;
    let pull_requests = fetch_pull_requests(&snapshot.sessions).await;
    let turn_kinds = user_turns(snapshot.sessions.iter(), &snapshot.agent_config)
        .into_iter()
        .map(|turn| (turn.session.session_id.as_str(), turn.kind))
        .collect::<HashMap<_, _>>();
    let now = Utc::now();

    let mut sessions = snapshot
        .sessions
        .iter()
        .map(|session| {
            let task = task_links.get(&session.session_id).map(|task| TqTaskView {
                id: task.id.clone(),
                number: task.number,
                title: task.title.clone(),
                priority: task_priorities.get(&task.id).copied().flatten(),
            });
            let parent = session
                .ancestor_session_ids
                .last()
                .map(|id| ParentSessionView {
                    id: id.clone(),
                    title: snapshot.title_by_id.get(id).cloned(),
                });
            let pull_request = pull_requests
                .get(&session.session_id)
                .and_then(Option::as_ref)
                .map(|info| PullRequestView {
                    number: info.number,
                    state: info.state.clone(),
                });

            SessionView {
                id: session.session_id.clone(),
                engine: session.engine,
                category: category_for(
                    session,
                    turn_kinds.get(session.session_id.as_str()).copied(),
                ),
                kind: kind_for(
                    session,
                    turn_kinds.get(session.session_id.as_str()).copied(),
                ),
                title: snapshot
                    .title_by_id
                    .get(&session.session_id)
                    .cloned()
                    .unwrap_or_default(),
                preview: session.last_message.clone(),
                updated_at: session.updated_at,
                elapsed_seconds: now
                    .signed_duration_since(session.updated_at)
                    .num_seconds()
                    .max(0) as u64,
                unread: session.is_unread_stopped(),
                task,
                repo: snapshot
                    .repo_by_id
                    .get(&session.session_id)
                    .cloned()
                    .unwrap_or_default(),
                parent,
                pull_request,
            }
        })
        .collect::<Vec<_>>();

    sort_session_views(&mut sessions);

    Ok(Json(SessionsResponse {
        revision: snapshot.revision,
        sessions,
    }))
}

fn load_snapshot() -> anyhow::Result<SessionSnapshot> {
    let revision = store::session_store_revision()?;
    let sessions = store::list_sessions()?;
    let title_by_id = store::list_all_sessions()?
        .into_iter()
        .map(|session| (session.session_id.clone(), title_display_name(&session)))
        .collect();
    let repo_by_id = sessions
        .iter()
        .map(|session| (session.session_id.clone(), resolve_repo(&session.cwd)))
        .collect();

    Ok(SessionSnapshot {
        revision,
        sessions,
        title_by_id,
        repo_by_id,
        agent_config: config::load_config_or_default().agent,
    })
}

fn resolve_repo(cwd: &std::path::Path) -> String {
    if let Ok(repo) = open_repo_at(cwd)
        && let Ok((owner, repo)) = github_owner_and_repo(&repo)
    {
        return format!("{owner}/{repo}");
    }

    cwd.file_name()
        .and_then(|name| name.to_str())
        .map(String::from)
        .unwrap_or_else(|| cwd.display().to_string())
}

async fn fetch_task_links(sessions: &[Session]) -> HashMap<String, TaskLink> {
    let session_ids = sessions
        .iter()
        .map(|session| session.session_id.clone())
        .collect::<HashSet<_>>();
    let Some(client) = detected_tq_client().await else {
        return HashMap::new();
    };
    let Ok(mut tq_sessions) = client.list_session_tasks(&session_ids).await else {
        return HashMap::new();
    };

    let local_ids = session_ids;
    let returned_ids = tq_sessions
        .iter()
        .map(|session| session.session_id.clone())
        .collect::<HashSet<_>>();
    let missing_parent_ids = tq_sessions
        .iter()
        .filter(|session| local_ids.contains(&session.session_id))
        .filter_map(|session| session.parent_session_id.clone())
        .filter(|parent_id| !returned_ids.contains(parent_id))
        .collect::<HashSet<_>>();
    if !missing_parent_ids.is_empty()
        && let Ok(parent_sessions) = client.list_session_tasks(&missing_parent_ids).await
    {
        tq_sessions.extend(
            parent_sessions
                .into_iter()
                .filter(|session| missing_parent_ids.contains(&session.session_id)),
        );
    }

    let inherited_task_ids = tq_sessions
        .iter()
        .filter(|session| local_ids.contains(&session.session_id))
        .filter_map(|session| session.parent_session_id.as_ref())
        .filter_map(|parent_id| {
            tq_sessions
                .iter()
                .find(|item| &item.session_id == parent_id)
        })
        .map(|parent| {
            (
                parent.session_id.clone(),
                parent
                    .tasks
                    .iter()
                    .map(|task| task.id.clone())
                    .collect::<HashSet<_>>(),
            )
        })
        .collect::<HashMap<_, _>>();

    tq_sessions
        .into_iter()
        .filter(|session| local_ids.contains(&session.session_id))
        .filter_map(|session| {
            let inherited_ids = session
                .parent_session_id
                .as_deref()
                .and_then(|parent_id| inherited_task_ids.get(parent_id));
            let task = session
                .tasks
                .into_iter()
                .min_by(|left, right| compare_tq_tasks(left, right, inherited_ids));
            task.map(|task| {
                (
                    session.session_id,
                    TaskLink {
                        id: task.id,
                        number: task.number,
                        title: crate::commands::agent::claude_sessions::normalize_title(
                            &task.title,
                        ),
                    },
                )
            })
        })
        .collect()
}

fn compare_tq_tasks(
    left: &TqTask,
    right: &TqTask,
    inherited_ids: Option<&HashSet<String>>,
) -> std::cmp::Ordering {
    let inherited_order = inherited_ids.map_or(std::cmp::Ordering::Equal, |ids| {
        ids.contains(&left.id).cmp(&ids.contains(&right.id))
    });
    inherited_order
        .then_with(|| {
            (left.status == TqTaskStatus::Completed).cmp(&(right.status == TqTaskStatus::Completed))
        })
        .then_with(|| right.linked_at.cmp(&left.linked_at))
}

async fn fetch_task_priorities(
    task_links: &HashMap<String, TaskLink>,
) -> HashMap<String, Option<i32>> {
    let task_ids = task_links
        .values()
        .map(|task| task.id.clone())
        .collect::<HashSet<_>>();
    let Some(client) = detected_tq_client().await else {
        return HashMap::new();
    };
    let Ok(tasks) = client.list_tasks(&task_ids).await else {
        return HashMap::new();
    };

    tasks
        .into_iter()
        .map(|task: TqTaskSummary| (task.id, task.priority))
        .collect()
}

async fn detected_tq_client() -> Option<TqClient> {
    tokio::task::spawn_blocking(TqClient::detect)
        .await
        .ok()
        .flatten()
}

async fn fetch_pull_requests(sessions: &[Session]) -> HashMap<String, Option<PrInfo>> {
    let mut query_by_session = HashMap::new();
    let mut queries = Vec::new();
    let mut seen = HashSet::new();
    for session in sessions {
        let Some(context) = session.pr_context.as_ref() else {
            continue;
        };
        let query = BranchPrQuery {
            owner: context.owner.clone(),
            repo: context.repo.clone(),
            branch: context.branch.clone(),
        };
        if seen.insert(query.key()) {
            queries.push(query.clone());
        }
        query_by_session.insert(session.session_id.clone(), query);
    }
    if queries.is_empty() {
        return HashMap::new();
    }
    let Ok(Ok(client)) = tokio::task::spawn_blocking(GitHubClient::get).await else {
        return HashMap::new();
    };
    let Ok(prs) = client.get_prs_for_branches_batch(&queries).await else {
        return HashMap::new();
    };

    query_by_session
        .into_iter()
        .map(|(session_id, query)| (session_id, prs.get(&query.key()).cloned().unwrap_or(None)))
        .collect()
}

fn category_for(session: &Session, turn: Option<UserTurnKind>) -> SessionCategory {
    if turn.is_some()
        || session.status == SessionStatus::Stopped
        || session
            .current_tool
            .as_deref()
            .is_some_and(|tool| tool.starts_with("ExitPlanMode"))
    {
        SessionCategory::NeedsAction
    } else if session.status == SessionStatus::Running {
        SessionCategory::InProgress
    } else {
        SessionCategory::Done
    }
}

fn kind_for(session: &Session, turn: Option<UserTurnKind>) -> Option<SessionKind> {
    if session
        .current_tool
        .as_deref()
        .is_some_and(|tool| tool.starts_with("ExitPlanMode"))
    {
        return Some(SessionKind::Plan);
    }

    let Some(turn) = turn else {
        return (session.status == SessionStatus::Stopped).then_some(SessionKind::Stopped);
    };

    match turn {
        UserTurnKind::Ask => Some(SessionKind::Question),
        UserTurnKind::Allow => Some(SessionKind::Approve),
        UserTurnKind::Crit | UserTurnKind::Body => Some(SessionKind::Review),
        UserTurnKind::Failed => Some(SessionKind::Error),
        UserTurnKind::AgentStatus => {
            let status = session.agent_status.as_ref()?;
            let content = format!("{} {}", status.kind, status.note).to_ascii_lowercase();
            if content.contains('?')
                || content.contains("question")
                || content.contains("ask")
                || content.contains("answer")
            {
                Some(SessionKind::Question)
            } else if content.contains("review")
                || content.contains("crit")
                || content.contains("body")
                || content.contains("approve")
                || content.contains("proposal")
            {
                Some(SessionKind::Review)
            } else {
                Some(SessionKind::Stopped)
            }
        }
        UserTurnKind::Merge => None,
    }
}

fn compare_tasks(left: &Option<TqTaskView>, right: &Option<TqTaskView>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left
            .priority
            .unwrap_or(i32::MAX)
            .cmp(&right.priority.unwrap_or(i32::MAX)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn sort_session_views(sessions: &mut [SessionView]) {
    sessions.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| compare_tasks(&left.task, &right.task))
            .then_with(|| left.updated_at.cmp(&right.updated_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{AgentStatus, TmuxInfo};
    use chrono::TimeZone;
    use rstest::rstest;
    use std::path::PathBuf;

    fn session(status: SessionStatus) -> Session {
        Session {
            session_id: "session".to_string(),
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/workspace/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None::<TmuxInfo>,
            status,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: Utc.timestamp_opt(1_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_000, 0).unwrap(),
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            launched_with_vibe: false,
            work_type_pinned: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }

    fn row(
        id: &str,
        category: SessionCategory,
        task: Option<(i32, u32)>,
        updated_at: i64,
    ) -> SessionView {
        SessionView {
            id: id.to_string(),
            engine: Engine::Claude,
            category,
            kind: None,
            title: id.to_string(),
            preview: None,
            updated_at: Utc.timestamp_opt(updated_at, 0).unwrap(),
            elapsed_seconds: 0,
            unread: false,
            task: task.map(|(priority, number)| TqTaskView {
                id: format!("task-{number}"),
                number,
                title: id.to_string(),
                priority: Some(priority),
            }),
            repo: String::new(),
            parent: None,
            pull_request: None,
        }
    }

    #[rstest]
    #[case::waiting_tool(UserTurnKind::Ask, Some(SessionKind::Question))]
    #[case::permission(UserTurnKind::Allow, Some(SessionKind::Approve))]
    #[case::crit_review(UserTurnKind::Crit, Some(SessionKind::Review))]
    #[case::draft_review(UserTurnKind::Body, Some(SessionKind::Review))]
    #[case::failure(UserTurnKind::Failed, Some(SessionKind::Error))]
    #[case::merge(UserTurnKind::Merge, None)]
    fn maps_user_turn_kinds_to_web_kinds(
        #[case] turn: UserTurnKind,
        #[case] expected: Option<SessionKind>,
    ) {
        assert_eq!(
            kind_for(&session(SessionStatus::WaitingInput), Some(turn)),
            expected
        );
    }

    #[rstest]
    #[case::agent_question("ask_user", "", SessionKind::Question)]
    #[case::note_question("needs_attention", "Which option?", SessionKind::Question)]
    #[case::agent_review("needs_review", "", SessionKind::Review)]
    #[case::agent_stopped("waiting_for_external", "", SessionKind::Stopped)]
    fn maps_agent_status_to_web_kind(
        #[case] status_kind: &str,
        #[case] note: &str,
        #[case] expected: SessionKind,
    ) {
        let mut session = session(SessionStatus::Stopped);
        session.agent_status = Some(AgentStatus {
            kind: status_kind.to_string(),
            note: note.to_string(),
        });

        assert_eq!(
            kind_for(&session, Some(UserTurnKind::AgentStatus)),
            Some(expected)
        );
    }

    #[test]
    fn maps_exit_plan_mode_to_plan_kind() {
        let mut session = session(SessionStatus::WaitingInput);
        session.current_tool = Some("ExitPlanMode".to_string());

        assert_eq!(
            kind_for(&session, Some(UserTurnKind::Ask)),
            Some(SessionKind::Plan)
        );
    }

    #[test]
    fn exit_plan_mode_needs_action() {
        let mut session = session(SessionStatus::WaitingInput);
        session.current_tool = Some("ExitPlanMode".to_string());

        assert_eq!(category_for(&session, None), SessionCategory::NeedsAction);
    }

    #[test]
    fn stopped_without_a_user_turn_has_stopped_kind() {
        assert_eq!(
            kind_for(&session(SessionStatus::Stopped), None),
            Some(SessionKind::Stopped)
        );
    }

    #[rstest]
    #[case::stopped(SessionStatus::Stopped, None, SessionCategory::NeedsAction)]
    #[case::user_turn(
        SessionStatus::WaitingInput,
        Some(UserTurnKind::Ask),
        SessionCategory::NeedsAction
    )]
    #[case::running(SessionStatus::Running, None, SessionCategory::InProgress)]
    #[case::paused(SessionStatus::Paused, None, SessionCategory::Done)]
    #[case::ended(SessionStatus::Ended, None, SessionCategory::Done)]
    fn maps_sessions_to_inbox_categories(
        #[case] status: SessionStatus,
        #[case] turn: Option<UserTurnKind>,
        #[case] expected: SessionCategory,
    ) {
        assert_eq!(category_for(&session(status), turn), expected);
    }

    #[rstest]
    #[case::lower_priority_first(Some((Some(0), 1)), Some((Some(1), 2)), std::cmp::Ordering::Less)]
    #[case::no_priority_after_prioritized(Some((Some(0), 1)), Some((None, 2)), std::cmp::Ordering::Less)]
    #[case::same_priority_ties(Some((Some(1), 1)), Some((Some(1), 2)), std::cmp::Ordering::Equal)]
    #[case::taskless_last(Some((Some(1), 1)), None, std::cmp::Ordering::Less)]
    fn compares_task_priority(
        #[case] left: Option<(Option<i32>, u32)>,
        #[case] right: Option<(Option<i32>, u32)>,
        #[case] expected: std::cmp::Ordering,
    ) {
        let to_task = |task: Option<(Option<i32>, u32)>| {
            task.map(|(priority, number)| TqTaskView {
                id: "task".to_string(),
                number,
                title: "task".to_string(),
                priority,
            })
        };
        assert_eq!(compare_tasks(&to_task(left), &to_task(right)), expected);
    }

    #[rstest]
    #[case::category_then_priority(
        vec![
            row("done", SessionCategory::Done, Some((0, 1)), 1),
            row("working", SessionCategory::InProgress, Some((0, 2)), 1),
            row("needs_low", SessionCategory::NeedsAction, Some((2, 3)), 1),
            row("needs_high", SessionCategory::NeedsAction, Some((1, 4)), 1),
            row("needs_without_task", SessionCategory::NeedsAction, None, 0),
        ],
        vec!["needs_high", "needs_low", "needs_without_task", "working", "done"]
    )]
    #[case::oldest_waiting_first(
        vec![
            row("newer", SessionCategory::NeedsAction, Some((1, 1)), 20),
            row("older", SessionCategory::NeedsAction, Some((1, 2)), 10),
        ],
        vec!["older", "newer"]
    )]
    fn orders_session_rows(#[case] mut rows: Vec<SessionView>, #[case] expected_ids: Vec<&str>) {
        sort_session_views(&mut rows);

        assert_eq!(
            rows.into_iter()
                .map(|session| session.id)
                .collect::<Vec<_>>(),
            expected_ids
        );
    }
}
