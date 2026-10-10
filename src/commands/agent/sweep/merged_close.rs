use std::collections::{HashMap, HashSet};
use std::future::Future;

use anyhow::Result;

use super::super::session_status::list_sessions_with_bg_run_status;
use super::super::types::{Session, SessionStatus};
use crate::infra::git::{GitRepo, github_owner_and_repo};
use crate::infra::github::{BranchPrQuery, GitHubClient, PrInfo, PrState};
use crate::infra::notification::{Notification, merge_icon};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MergedSession {
    session_id: String,
    branch: String,
    pr: PrInfo,
}

pub(super) async fn close_merged_sessions(dry_run: bool) -> usize {
    let sessions = match list_sessions_with_bg_run_status() {
        Ok(sessions) => sessions,
        Err(error) => {
            tracing::warn!(event = "agent.sweep.sessions_read_failed", error = %error);
            return 0;
        }
    };

    let mut queries_by_session = HashMap::new();
    let mut queries = Vec::new();
    let mut seen_queries = HashSet::new();

    for session in &sessions {
        if !is_close_eligible(session) {
            continue;
        }

        let Some(query) = branch_query_for(session) else {
            continue;
        };
        if seen_queries.insert(query.key()) {
            queries.push(query.clone());
        }
        queries_by_session.insert(session.session_id.clone(), query);
    }

    if queries.is_empty() {
        return 0;
    }

    let client = match GitHubClient::get() {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(event = "agent.sweep.github_client_unavailable", error = %error);
            return 0;
        }
    };
    let prs = match client.get_prs_for_branches_batch(&queries).await {
        Ok(prs) => prs,
        Err(error) => {
            tracing::warn!(event = "agent.sweep.pull_requests_fetch_failed", error = %error);
            return 0;
        }
    };
    let closed = close_merged_sessions_with(
        &sessions,
        &queries_by_session,
        &prs,
        dry_run,
        |args| async move { super::super::close::run(&args).await },
    );
    let closed = closed.await;

    let icon = if closed.is_empty() {
        None
    } else {
        merge_icon::ensure_icon().await
    };
    for candidate in &closed {
        let notification = build_notification(candidate, icon.as_deref());
        if let Err(error) = crate::infra::notification::send(&notification) {
            tracing::warn!(
                event = "agent.sweep.merge_notification_failed",
                session = %candidate.session_id,
                error = %error,
            );
        }
    }

    closed.len()
}

fn branch_query_for(session: &Session) -> Option<BranchPrQuery> {
    let repo = GitRepo::open_at(&session.cwd).ok()?;
    branch_query_from_repository(
        repo.is_worktree(),
        || repo.current_branch().ok(),
        || github_owner_and_repo(&repo).ok(),
    )
}

fn branch_query_from_repository(
    is_worktree: bool,
    branch: impl FnOnce() -> Option<String>,
    repository: impl FnOnce() -> Option<(String, String)>,
) -> Option<BranchPrQuery> {
    if !is_worktree {
        return None;
    }
    let branch = branch()?;
    if branch == "HEAD" {
        return None;
    }
    let (owner, repo) = repository()?;
    Some(BranchPrQuery {
        owner,
        repo,
        branch,
    })
}

fn is_close_eligible(session: &Session) -> bool {
    matches!(
        session.status,
        SessionStatus::Stopped | SessionStatus::Paused
    ) && !session.has_pending_bg_tasks()
}

pub(super) async fn close_merged_sessions_with<F, Fut>(
    sessions: &[Session],
    queries_by_session: &HashMap<String, BranchPrQuery>,
    prs: &HashMap<(String, String, String), Option<PrInfo>>,
    dry_run: bool,
    close: F,
) -> Vec<MergedSession>
where
    F: FnMut(super::super::close::CloseArgs) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let candidates: Vec<_> = sessions
        .iter()
        .filter_map(|session| {
            if !is_close_eligible(session) {
                return None;
            }
            let query = queries_by_session.get(&session.session_id)?;
            let pr = prs.get(&query.key())?.as_ref()?;
            if pr.state != PrState::Merged {
                return None;
            }
            Some(MergedSession {
                session_id: session.session_id.clone(),
                branch: query.branch.clone(),
                pr: pr.clone(),
            })
        })
        .collect();
    if dry_run {
        for candidate in &candidates {
            tracing::info!(
                event = "agent.sweep.dry_run_close_merged",
                session = %candidate.session_id,
                branch = %candidate.branch,
                pr = candidate.pr.number,
            );
            eprintln!(
                "[armyknife] agent sweep (dry-run): would close {} after merged PR #{}",
                candidate.session_id, candidate.pr.number,
            );
        }
        return Vec::new();
    }
    close_candidates(candidates, close).await
}

async fn close_candidates<F, Fut>(
    candidates: Vec<MergedSession>,
    mut close: F,
) -> Vec<MergedSession>
where
    F: FnMut(super::super::close::CloseArgs) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut closed = Vec::new();
    for candidate in candidates {
        match close(close_args(&candidate.session_id)).await {
            Ok(()) => closed.push(candidate),
            Err(error) => tracing::warn!(
                event = "agent.sweep.merged_session_close_failed",
                session = %candidate.session_id,
                pr = candidate.pr.number,
                error = %error,
            ),
        }
    }
    closed
}

fn close_args(session_id: &str) -> super::super::close::CloseArgs {
    super::super::close::CloseArgs {
        target: Some(session_id.to_string()),
        force: false,
        skip_hooks: false,
    }
}

pub(super) fn build_notification(
    candidate: &MergedSession,
    icon_path: Option<&std::path::Path>,
) -> Notification {
    let title = sanitize_title(&candidate.pr.title);
    let mut notification = Notification::new(
        "tmux PR merge succeeded",
        format!(
            "PR #{} ({title}) was merged and the agent session was closed.",
            candidate.pr.number,
        ),
    )
    .with_subtitle(format!("PR #{} {title}", candidate.pr.number));
    if let Some(icon_path) = icon_path {
        notification = notification.with_app_icon(icon_path.to_string_lossy());
    }
    notification
}

fn sanitize_title(title: &str) -> String {
    title
        .chars()
        .map(|character| {
            if matches!(character, '\r' | '\n') {
                ' '
            } else {
                character
            }
        })
        .filter(|character| !character.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use chrono::{DateTime, Utc};
    use indoc::indoc;
    use rstest::rstest;

    use super::*;
    use crate::infra::github::PrState;
    use crate::infra::notification::Notification;

    fn make_session(id: &str, status: SessionStatus) -> Session {
        let timestamp: DateTime<Utc> = Utc::now();
        Session {
            session_id: id.to_string(),
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/worktree"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: timestamp,
            updated_at: timestamp,
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            work_type_pinned: false,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: BTreeMap::new(),
            read_at: None,
            sweep_signaled: false,
            agent_status: None,
            engine: Default::default(),
        }
    }

    fn make_query(branch: &str) -> BranchPrQuery {
        BranchPrQuery {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            branch: branch.to_string(),
        }
    }

    fn make_pr(number: u64, state: PrState) -> PrInfo {
        PrInfo {
            number,
            title: format!("Change {number}"),
            state,
            url: format!("https://github.com/owner/repo/pull/{number}"),
        }
    }

    #[tokio::test]
    async fn selects_only_idle_linked_worktree_sessions_with_merged_prs() {
        let mut sessions = vec![
            make_session("stopped-merged", SessionStatus::Stopped),
            make_session("paused-merged", SessionStatus::Paused),
            make_session("running-merged", SessionStatus::Running),
            make_session("waiting-merged", SessionStatus::WaitingInput),
            make_session("ended-merged", SessionStatus::Ended),
            make_session("pending-task", SessionStatus::Stopped),
            make_session("pending-bg-run", SessionStatus::Paused),
            make_session("no-worktree", SessionStatus::Stopped),
            make_session("open-pr", SessionStatus::Stopped),
            make_session("closed-pr", SessionStatus::Stopped),
            make_session("no-pr", SessionStatus::Stopped),
        ];
        sessions[5]
            .pending_bg_task_ids
            .insert("task-id".to_string());
        sessions[6]
            .pending_bg_task_ids
            .insert(super::super::super::types::BG_RUN_PENDING_TASK_MARKER.to_string());

        let queries_by_session: HashMap<_, _> = sessions
            .iter()
            .filter(|session| session.session_id != "no-worktree")
            .map(|session| (session.session_id.clone(), make_query(&session.session_id)))
            .collect();
        let prs: HashMap<_, _> = [
            ("stopped-merged", Some(make_pr(1, PrState::Merged))),
            ("paused-merged", Some(make_pr(2, PrState::Merged))),
            ("running-merged", Some(make_pr(3, PrState::Merged))),
            ("waiting-merged", Some(make_pr(4, PrState::Merged))),
            ("ended-merged", Some(make_pr(5, PrState::Merged))),
            ("pending-task", Some(make_pr(6, PrState::Merged))),
            ("pending-bg-run", Some(make_pr(7, PrState::Merged))),
            ("open-pr", Some(make_pr(8, PrState::Open))),
            ("closed-pr", Some(make_pr(9, PrState::Closed))),
            ("no-pr", None),
        ]
        .into_iter()
        .map(|(session_id, pr)| {
            let query = &queries_by_session[session_id];
            (query.key(), pr)
        })
        .collect();

        assert_eq!(
            close_merged_sessions_with(
                &sessions,
                &queries_by_session,
                &prs,
                false,
                |_args| async move { Ok(()) },
            )
            .await,
            vec![
                MergedSession {
                    session_id: "stopped-merged".to_string(),
                    branch: "stopped-merged".to_string(),
                    pr: make_pr(1, PrState::Merged),
                },
                MergedSession {
                    session_id: "paused-merged".to_string(),
                    branch: "paused-merged".to_string(),
                    pr: make_pr(2, PrState::Merged),
                },
            ],
        );
    }

    #[tokio::test]
    async fn close_failure_does_not_skip_later_merged_sessions() {
        let sessions = vec![
            make_session("failing-session", SessionStatus::Paused),
            make_session("successful-session", SessionStatus::Stopped),
        ];
        let queries_by_session: HashMap<_, _> = sessions
            .iter()
            .map(|session| (session.session_id.clone(), make_query(&session.session_id)))
            .collect();
        let prs: HashMap<_, _> = [
            ("failing-session", Some(make_pr(11, PrState::Merged))),
            ("successful-session", Some(make_pr(12, PrState::Merged))),
        ]
        .into_iter()
        .map(|(session_id, pr)| {
            let query = &queries_by_session[session_id];
            (query.key(), pr)
        })
        .collect();
        let calls = RefCell::new(Vec::new());
        let closed =
            close_merged_sessions_with(&sessions, &queries_by_session, &prs, false, |args| {
                let calls = &calls;
                async move {
                    let session_id = args.target.clone().unwrap_or_default();
                    calls
                        .borrow_mut()
                        .push((args.target.clone(), args.force, args.skip_hooks));
                    if session_id == "failing-session" {
                        anyhow::bail!("close refused");
                    }
                    Ok(())
                }
            })
            .await;

        assert_eq!(
            (calls.into_inner(), closed),
            (
                vec![
                    (Some("failing-session".to_string()), false, false),
                    (Some("successful-session".to_string()), false, false),
                ],
                vec![MergedSession {
                    session_id: "successful-session".to_string(),
                    branch: "successful-session".to_string(),
                    pr: make_pr(12, PrState::Merged),
                }],
            ),
        );
    }

    #[tokio::test]
    async fn dry_run_does_not_call_close() {
        let sessions = vec![make_session("merged-session", SessionStatus::Paused)];
        let queries_by_session =
            HashMap::from([("merged-session".to_string(), make_query("merged-session"))]);
        let query = &queries_by_session["merged-session"];
        let prs = HashMap::from([(query.key(), Some(make_pr(14, PrState::Merged)))]);
        let calls = RefCell::new(Vec::new());
        let closed =
            close_merged_sessions_with(&sessions, &queries_by_session, &prs, true, |args| {
                calls
                    .borrow_mut()
                    .push((args.target, args.force, args.skip_hooks));
                async { Ok(()) }
            })
            .await;

        assert_eq!((calls.into_inner(), closed), (vec![], vec![]));
    }

    #[rstest]
    #[case::without_icon(
        None,
        Notification::new(
            "tmux PR merge succeeded",
            "PR #15 (A merged change) was merged and the agent session was closed.",
        )
        .with_subtitle("PR #15 A merged change"),
    )]
    #[case::with_icon(
        Some("/tmp/merge-icon.png"),
        Notification::new(
            "tmux PR merge succeeded",
            "PR #15 (A merged change) was merged and the agent session was closed.",
        )
        .with_subtitle("PR #15 A merged change")
        .with_app_icon("/tmp/merge-icon.png"),
    )]
    fn notification_cases(#[case] icon_path: Option<&str>, #[case] expected: Notification) {
        let candidate = MergedSession {
            session_id: "sample-session".to_string(),
            branch: "sample-branch".to_string(),
            pr: PrInfo {
                number: 15,
                title: "A merged change".to_string(),
                state: PrState::Merged,
                url: "https://github.com/owner/repo/pull/15".to_string(),
            },
        };
        assert_eq!(
            build_notification(&candidate, icon_path.map(std::path::Path::new),),
            expected,
        );
    }

    #[rstest]
    #[case::single_line("A merged change", "A merged change")]
    #[case::multiline(
        indoc! {"
            A merged
            change"},
        "A merged change"
    )]
    fn sanitizes_notification_title(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(sanitize_title(input), expected);
    }

    #[rstest]
    #[case::main_checkout(false, Some("feature"), None)]
    #[case::linked_worktree(
        true,
        Some("feature"),
        Some(BranchPrQuery {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            branch: "feature".to_string(),
        })
    )]
    #[case::detached_worktree(true, Some("HEAD"), None)]
    #[case::branch_unavailable(true, None, None)]
    fn branch_query_requires_a_linked_worktree_branch(
        #[case] is_worktree: bool,
        #[case] branch: Option<&str>,
        #[case] expected: Option<BranchPrQuery>,
    ) {
        let actual = branch_query_from_repository(
            is_worktree,
            || branch.map(str::to_string),
            || Some(("owner".to_string(), "repo".to_string())),
        );

        assert_eq!(actual, expected);
    }
}
