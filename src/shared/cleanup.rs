//! Shared cleanup logic for agent sessions, notifications, and git worktrees.
//!
//! Both `agent watch` (session cleanup) and `agent close`/`agent clean` (worktree cleanup)
//! need to clean up related resources. This module provides the shared logic to
//! ensure consistent cleanup regardless of the entry point.

use std::collections::HashSet;
use std::path::Path;

use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session, SessionStatus};
use crate::infra::tmux;
mod worktree;
pub(crate) use worktree::check_process_guard;
pub(crate) use worktree::cleanup_worktree_by_name_with_before_delete;
pub use worktree::{
    WorktreeCleanupResult, WorktreeProcessGuardError, cleanup_worktree_by_name,
    cleanup_worktree_resources, cleanup_worktree_resources_with_post_delete,
};

/// Returns true if the pane hosting this session still runs a live agent
/// process that needs SIGTERM before we delete the session record.
///
/// Paused sessions were already SIGTERM'd by `agent sweep`, and Ended sessions
/// exited on their own. In both cases the pane's foreground process is the
/// user's shell (possibly the shell that just invoked `a agent close` from
/// within the worktree). Signaling that shell would kill the very process we
/// are running in, so restrict SIGTERM to statuses where a Claude process is
/// still expected to be alive.
fn should_sigterm_session(status: SessionStatus) -> bool {
    match status {
        SessionStatus::Running
        | SessionStatus::WaitingInput
        | SessionStatus::Stopped
        | SessionStatus::Failed => true,
        SessionStatus::Paused | SessionStatus::Ended => false,
    }
}

/// Cleans up active agent session records and notification groups for sessions
/// whose `cwd` is inside `worktree_path`.
///
/// Every stored status is considered so `Ended` sessions with stale notifications
/// are included. Ended session records are retained for later delegated-session lookup.
///
/// For each matching session:
/// 1. Archives Codex threads, except the current thread, which is archived
///    after this cleanup process exits.
/// 2. If the session's Claude process is still expected to be alive and the
///    tmux pane is alive, sends SIGTERM to it
/// 3. Attempts to withdraw unanswered todos and proposals; failures are warned and cleanup continues
/// 4. Removes the notification group on a best-effort basis
///
/// Returns the number of sessions cleaned up.
pub fn cleanup_sessions_in_path(worktree_path: &Path) -> anyhow::Result<usize> {
    cleanup_sessions_in_path_with_policy(worktree_path, true)
}

fn cleanup_sessions_in_path_with_policy(
    worktree_path: &Path,
    signal_agents: bool,
) -> anyhow::Result<usize> {
    let sessions = store::list_all_sessions()?;
    let alive_panes_by_socket = if signal_agents {
        cache_alive_panes_by_socket(&sessions, worktree_path, |socket_path| {
            tmux::list_all_pane_ids_for_socket(socket_path).map_err(anyhow::Error::from)
        })
    } else {
        tmux::PaneLiveness::default()
    };
    let own_codex_session_id = crate::shared::env_var::EnvVars::load().codex_session_id;

    Ok(cleanup_sessions_with_deferred_archive(
        &sessions,
        SessionCleanupContext {
            worktree_path,
            alive_panes_by_socket: &alive_panes_by_socket,
            own_codex_session_id: own_codex_session_id.as_deref(),
        },
        SessionCleanupOperations {
            send_sigterm: tmux::send_sigterm_to_pane_for_socket,
            delete_session: store::delete_session_without_archive,
            withdraw_pending_cards: |session_id: &str| match store::sessions_dir() {
                Ok(sessions_dir) => withdraw_pending_cards(
                    &sessions_dir,
                    session_id,
                    "Withdrawn during worktree cleanup.",
                    store::WithdrawalReason::WorktreeCleanup,
                ),
                Err(error) => PendingCardCleanup::failed(error),
            },
            remove_notification_group: crate::infra::notification::remove_group,
            archive_codex_thread: crate::commands::agent::codex_steer::archive_thread,
            defer_archive: crate::commands::agent::spawn_after_parent_exit,
        },
    ))
}

type AlivePanesBySocket = tmux::PaneLiveness;

fn cache_alive_panes_by_socket(
    sessions: &[Session],
    worktree_path: &Path,
    mut list_panes: impl FnMut(Option<&str>) -> anyhow::Result<HashSet<String>>,
) -> AlivePanesBySocket {
    let mut alive_panes_by_socket = tmux::PaneLiveness::default();

    for socket_path in sessions
        .iter()
        .filter(|session| {
            session.cwd.starts_with(worktree_path) && should_sigterm_session(session.status)
        })
        .filter_map(|session| session.tmux_info.as_ref())
        .map(|info| info.socket_path.clone())
    {
        alive_panes_by_socket.cache_socket(socket_path.as_deref(), &mut list_panes);
    }

    alive_panes_by_socket
}

struct SessionCleanupOperations<
    SendSigterm,
    DeleteSession,
    WithdrawCards,
    RemoveNotification,
    Archive,
    DeferArchive,
> {
    send_sigterm: SendSigterm,
    delete_session: DeleteSession,
    withdraw_pending_cards: WithdrawCards,
    remove_notification_group: RemoveNotification,
    archive_codex_thread: Archive,
    defer_archive: DeferArchive,
}

struct PendingCardCleanup {
    _todo_resolution_lock: Option<std::fs::File>,
    error: Option<anyhow::Error>,
}

impl PendingCardCleanup {
    fn succeeded(todo_resolution_lock: std::fs::File) -> Self {
        Self {
            _todo_resolution_lock: Some(todo_resolution_lock),
            error: None,
        }
    }

    fn failed(error: anyhow::Error) -> Self {
        Self {
            _todo_resolution_lock: None,
            error: Some(error),
        }
    }
}

fn cleanup_sessions_with_deferred_archive<
    SendSigterm,
    DeleteSession,
    WithdrawCards,
    RemoveNotification,
    Archive,
    DeferArchive,
>(
    sessions: &[Session],
    context: SessionCleanupContext<'_>,
    operations: SessionCleanupOperations<
        SendSigterm,
        DeleteSession,
        WithdrawCards,
        RemoveNotification,
        Archive,
        DeferArchive,
    >,
) -> usize
where
    SendSigterm: FnMut(Option<&str>, &str),
    DeleteSession: FnMut(&str) -> anyhow::Result<bool>,
    WithdrawCards: FnMut(&str) -> PendingCardCleanup,
    RemoveNotification: FnMut(&str) -> anyhow::Result<()>,
    Archive: FnMut(&str) -> anyhow::Result<()>,
    DeferArchive: FnMut(&str) -> anyhow::Result<()>,
{
    let deferred_archive = context
        .own_codex_session_id
        .filter(|session_id| {
            sessions.iter().any(|session| {
                session.engine == Engine::Codex
                    && session.session_id == *session_id
                    && session.cwd.starts_with(context.worktree_path)
            })
        })
        .map(str::to_string);

    let SessionCleanupOperations {
        send_sigterm,
        delete_session,
        withdraw_pending_cards,
        remove_notification_group,
        archive_codex_thread,
        mut defer_archive,
    } = operations;

    let cleaned = cleanup_sessions(
        sessions,
        context,
        send_sigterm,
        delete_session,
        withdraw_pending_cards,
        remove_notification_group,
        archive_codex_thread,
    );

    if let Some(session_id) = deferred_archive
        && let Err(error) = defer_archive(&session_id)
    {
        eprintln!(
            "Warning: Failed to defer Codex thread archive for session {session_id}: {error:#}"
        );
        tracing::warn!(
            target: "armyknife::shared::cleanup",
            event = "cleanup.codex_archive.defer_err",
            session = %session_id,
            msg = format!("failed to defer Codex thread archive: {error:#}"),
        );
    }

    cleaned
}

struct SessionCleanupContext<'a> {
    worktree_path: &'a Path,
    alive_panes_by_socket: &'a AlivePanesBySocket,
    own_codex_session_id: Option<&'a str>,
}

fn cleanup_sessions(
    sessions: &[Session],
    context: SessionCleanupContext<'_>,
    mut send_sigterm: impl FnMut(Option<&str>, &str),
    mut delete_session: impl FnMut(&str) -> anyhow::Result<bool>,
    mut withdraw_pending_cards: impl FnMut(&str) -> PendingCardCleanup,
    mut remove_notification_group: impl FnMut(&str) -> anyhow::Result<()>,
    mut archive_codex_thread: impl FnMut(&str) -> anyhow::Result<()>,
) -> usize {
    let mut cleaned = 0;

    for session in sessions {
        if session.cwd.starts_with(context.worktree_path) {
            let is_own_codex_session = session.engine == Engine::Codex
                && context.own_codex_session_id == Some(session.session_id.as_str());

            if session.engine == Engine::Codex && !is_own_codex_session {
                store::archive_codex_thread_before_delete_with(
                    session,
                    &mut archive_codex_thread,
                    store::ArchiveLogContext::WorktreeCleanup,
                );
            }

            if !is_own_codex_session
                && should_sigterm_session(session.status)
                && let Some(tmux_info) = session.tmux_info.as_ref()
                && context
                    .alive_panes_by_socket
                    .is_alive(tmux_info.socket_path.as_deref(), &tmux_info.pane_id)
                    == Some(true)
            {
                send_sigterm(tmux_info.socket_path.as_deref(), &tmux_info.pane_id);
            }

            let pending_card_cleanup = withdraw_pending_cards(&session.session_id);
            if let Some(error) = &pending_card_cleanup.error {
                eprintln!(
                    "Warning: Failed to withdraw pending cards for session {}: {error:#}",
                    session.session_id
                );
                tracing::warn!(
                    target: "armyknife::shared::cleanup",
                    event = "cleanup.pending_card_withdrawal.err",
                    session = %session.session_id,
                    msg = format!("failed to withdraw pending cards: {error:#}"),
                );
            }

            if session.status != SessionStatus::Ended {
                match delete_session(&session.session_id) {
                    Ok(true) => cleaned += 1,
                    Ok(false) => {}
                    Err(e) => {
                        eprintln!(
                            "Warning: Failed to delete session {}: {e}",
                            session.session_id
                        );
                    }
                }
            }

            drop(pending_card_cleanup);

            if let Err(e) = remove_notification_group(&session.session_id) {
                eprintln!(
                    "Warning: Failed to remove notification group for session {}: {e}",
                    session.session_id
                );
                tracing::warn!(
                    target: "armyknife::shared::cleanup",
                    event = "cleanup.notification.err",
                    session = %session.session_id,
                    msg = format!("failed to remove notification group: {e}"),
                );
            }
        }
    }

    cleaned
}

fn withdraw_pending_todos(
    sessions_dir: &Path,
    session_id: &str,
    comment: &str,
    reason: store::WithdrawalReason,
) -> anyhow::Result<()> {
    let todos = store::agent_todos_from(sessions_dir, session_id)?;
    let mut errors = Vec::new();

    for todo in todos.into_iter().filter(|todo| todo.resolution.is_none()) {
        if let Err(error) = store::resolve_agent_todo_from_with_reason(
            sessions_dir,
            session_id,
            todo.seq,
            store::TodoResolution::Withdrawn,
            Some(comment),
            reason,
        ) {
            errors.push(format!("todo {}: {error:#}", todo.seq));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(errors.join("; "))
    }
}

fn withdraw_pending_proposals(
    sessions_dir: &Path,
    session_id: &str,
    comment: &str,
    reason: store::WithdrawalReason,
) -> anyhow::Result<()> {
    let proposals = store::agent_proposals_from(sessions_dir, session_id)?;
    let mut errors = Vec::new();

    for proposal in proposals
        .into_iter()
        .filter(|proposal| proposal.resolved_at.is_none())
    {
        let result = (|| {
            let _lock =
                store::lock_agent_proposal_resolution_from(sessions_dir, session_id, proposal.seq)?;
            store::resolve_agent_proposal_from_with_reason(
                sessions_dir,
                session_id,
                proposal.seq,
                Some(comment),
                reason,
            )?;
            anyhow::Ok(())
        })();

        if let Err(error) = result {
            errors.push(format!("proposal {}: {error:#}", proposal.seq));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(errors.join("; "))
    }
}

fn withdraw_pending_cards(
    sessions_dir: &Path,
    session_id: &str,
    comment: &str,
    reason: store::WithdrawalReason,
) -> PendingCardCleanup {
    let todo_resolution_lock = match store::lock_agent_todo_resolution(sessions_dir, session_id) {
        Ok(lock) => lock,
        Err(error) => return PendingCardCleanup::failed(error),
    };
    let todo_result = withdraw_pending_todos(sessions_dir, session_id, comment, reason);
    let proposal_result = withdraw_pending_proposals(sessions_dir, session_id, comment, reason);
    let errors = [todo_result.err(), proposal_result.err()]
        .into_iter()
        .flatten()
        .map(|error| format!("{error:#}"))
        .collect::<Vec<_>>();

    if errors.is_empty() {
        PendingCardCleanup::succeeded(todo_resolution_lock)
    } else {
        PendingCardCleanup {
            _todo_resolution_lock: Some(todo_resolution_lock),
            error: Some(anyhow::anyhow!(errors.join("; "))),
        }
    }
}

pub(crate) fn withdraw_ended_session_cards(sessions_dir: &Path, session_id: &str) {
    let cleanup = withdraw_pending_cards(
        sessions_dir,
        session_id,
        "Withdrawn because the session ended.",
        store::WithdrawalReason::SessionEnded,
    );
    let PendingCardCleanup { error, .. } = cleanup;

    if let Some(error) = error {
        tracing::warn!(
            target: "armyknife::shared::cleanup",
            event = "cleanup.ended_session_card_withdrawal.err",
            session = %session_id,
            msg = format!("failed to withdraw pending cards: {error:#}"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{Engine, TmuxInfo};
    use chrono::Utc;
    use rstest::rstest;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_session(session_id: &str, cwd: &Path, engine: Engine) -> Session {
        Session {
            launch_id: None,
            archived_at: None,
            pr_context: None,
            created_pr_urls: Vec::new(),
            session_id: session_id.to_string(),
            crit_urls: Vec::new(),
            crit_not_waiting_urls: Default::default(),
            web_uis: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: cwd.to_path_buf(),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_prompted_at: None,
            last_message: None,
            current_tool: None,
            current_tool_name: None,
            label: None,
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine,
            model: None,
            effort: None,
        }
    }

    fn with_tmux_pane(mut session: Session, pane_id: &str, socket_path: Option<&str>) -> Session {
        session.tmux_info = Some(TmuxInfo {
            session_name: "test-session".to_string(),
            window_name: "test-window".to_string(),
            window_index: 0,
            pane_id: pane_id.to_string(),
            socket_path: socket_path.map(str::to_string),
        });
        session
    }

    #[test]
    fn a_preserved_session_is_not_counted_as_cleaned_up() {
        let worktree_path = PathBuf::from("/tmp/example-worktree");
        let sessions = vec![make_session("session-a", &worktree_path, Engine::Claude)];

        let cleaned = cleanup_sessions(
            &sessions,
            SessionCleanupContext {
                worktree_path: &worktree_path,
                alive_panes_by_socket: &tmux::PaneLiveness::default(),
                own_codex_session_id: None,
            },
            |_, _| {},
            |_| Ok(false),
            |_| PendingCardCleanup {
                _todo_resolution_lock: None,
                error: None,
            },
            |_| Ok(()),
            |_| Ok(()),
        );

        assert_eq!(cleaned, 0);
    }

    #[test]
    fn cleanup_withdraws_pending_todos_and_proposals_from_the_inbox() {
        let sessions_dir = TempDir::new().expect("temporary sessions directory should be created");
        let worktree_path = PathBuf::from("/tmp/example-worktree");
        let session = make_session("session-a", &worktree_path, Engine::Claude);
        store::save_session_to(sessions_dir.path(), &session).expect("session should be stored");

        let completed_todo = store::add_agent_todo_in(
            sessions_dir.path(),
            &session.session_id,
            "A finished task",
            None,
            false,
        )
        .expect("completed todo should be recorded");
        store::resolve_agent_todo_from(
            sessions_dir.path(),
            &session.session_id,
            completed_todo,
            store::TodoResolution::Completed,
            None,
        )
        .expect("todo should be completed");
        store::add_agent_todo_in(
            sessions_dir.path(),
            &session.session_id,
            "A pending task",
            None,
            false,
        )
        .expect("pending todo should be recorded");

        let answered_proposal = store::add_agent_proposal_in(
            sessions_dir.path(),
            &session.session_id,
            "{\"title\":\"An answered choice\"}",
        )
        .expect("answered proposal should be recorded");
        store::write_agent_proposal_answer_at_from(
            sessions_dir.path(),
            &session.session_id,
            answered_proposal,
            "Selected an option.",
            chrono::Utc::now(),
        )
        .expect("proposal should be answered");
        store::add_agent_proposal_in(
            sessions_dir.path(),
            &session.session_id,
            "{\"title\":\"A pending choice\"}",
        )
        .expect("pending proposal should be recorded");

        cleanup_sessions(
            std::slice::from_ref(&session),
            SessionCleanupContext {
                worktree_path: &worktree_path,
                alive_panes_by_socket: &tmux::PaneLiveness::default(),
                own_codex_session_id: None,
            },
            |_, _| {},
            |_| Ok(true),
            |session_id| {
                withdraw_pending_cards(
                    sessions_dir.path(),
                    session_id,
                    "Withdrawn during worktree cleanup.",
                    store::WithdrawalReason::WorktreeCleanup,
                )
            },
            |_| Ok(()),
            |_| Ok(()),
        );

        let todos = store::agent_todos_from(sessions_dir.path(), &session.session_id)
            .expect("todos should remain queryable");
        let proposals = store::agent_proposals_from(sessions_dir.path(), &session.session_id)
            .expect("proposals should remain queryable");
        let session_ids = vec![session.session_id];
        assert_eq!(
            (
                todos
                    .iter()
                    .map(|todo| {
                        (
                            todo.seq,
                            todo.resolution,
                            todo.withdrawal_reason,
                            todo.comment.as_deref(),
                        )
                    })
                    .collect::<Vec<_>>(),
                proposals
                    .iter()
                    .map(|proposal| {
                        (
                            proposal.seq,
                            proposal.resolution,
                            proposal.resolved_at.is_some(),
                            proposal.withdrawal_reason,
                            proposal.comment.as_deref(),
                        )
                    })
                    .collect::<Vec<_>>(),
                store::incomplete_agent_todo_titles_from(sessions_dir.path(), &session_ids)
                    .expect("incomplete todo titles should be queryable"),
                store::unanswered_agent_proposal_bodies_from(sessions_dir.path(), &session_ids)
                    .expect("unanswered proposal bodies should be queryable"),
            ),
            (
                vec![
                    (1, Some(store::TodoResolution::Completed), None, None),
                    (
                        2,
                        Some(store::TodoResolution::Withdrawn),
                        Some(store::WithdrawalReason::WorktreeCleanup),
                        Some("Withdrawn during worktree cleanup."),
                    ),
                ],
                vec![
                    (1, None, true, None, None),
                    (
                        2,
                        Some(store::ProposalResolution::Withdrawn),
                        true,
                        Some(store::WithdrawalReason::WorktreeCleanup),
                        Some("Withdrawn during worktree cleanup."),
                    ),
                ],
                vec![],
                vec![],
            ),
        );
    }

    #[test]
    fn cleanup_continues_after_pending_card_withdrawal_fails() {
        let worktree_path = PathBuf::from("/tmp/example-worktree");
        let session = make_session("session-a", &worktree_path, Engine::Claude);
        let events = RefCell::new(Vec::new());

        let cleaned = cleanup_sessions(
            &[session],
            SessionCleanupContext {
                worktree_path: &worktree_path,
                alive_panes_by_socket: &tmux::PaneLiveness::default(),
                own_codex_session_id: None,
            },
            |_, _| {},
            |session_id| {
                events.borrow_mut().push(format!("delete:{session_id}"));
                Ok(true)
            },
            |_| PendingCardCleanup::failed(anyhow::anyhow!("card database is unavailable")),
            |session_id| {
                events
                    .borrow_mut()
                    .push(format!("notification:{session_id}"));
                Ok(())
            },
            |_| Ok(()),
        );

        assert_eq!(
            (cleaned, events.into_inner()),
            (
                1,
                vec![
                    "delete:session-a".to_string(),
                    "notification:session-a".to_string(),
                ],
            ),
        );
    }

    #[test]
    fn cleanup_uses_each_recorded_socket_and_does_not_signal_when_listing_fails() {
        let worktree_path = PathBuf::from("/tmp/armyknife-test-worktree");
        let private_socket = "/tmp/armyknife-test-tmux-private";
        let unavailable_socket = "/tmp/armyknife-test-tmux-unavailable";
        let unrelated_socket = "/tmp/armyknife-test-tmux-unrelated";
        let sessions = vec![
            with_tmux_pane(
                make_session("legacy", &worktree_path, Engine::Claude),
                "%legacy",
                None,
            ),
            with_tmux_pane(
                make_session("private-a", &worktree_path, Engine::Claude),
                "%private-a",
                Some(private_socket),
            ),
            with_tmux_pane(
                make_session("private-b", &worktree_path, Engine::Claude),
                "%private-b",
                Some(private_socket),
            ),
            with_tmux_pane(
                make_session("unavailable", &worktree_path, Engine::Claude),
                "%unavailable",
                Some(unavailable_socket),
            ),
            with_tmux_pane(
                make_session(
                    "unrelated-worktree",
                    Path::new("/tmp/other-worktree"),
                    Engine::Claude,
                ),
                "%unrelated",
                Some(unrelated_socket),
            ),
        ];
        let queries = RefCell::new(Vec::new());
        let alive_panes_by_socket =
            cache_alive_panes_by_socket(&sessions, &worktree_path, |socket_path| {
                queries.borrow_mut().push(socket_path.map(str::to_string));
                match socket_path {
                    None => Ok(HashSet::from(["%legacy".to_string()])),
                    Some(path) if path == private_socket => Ok(HashSet::from([
                        "%private-a".to_string(),
                        "%private-b".to_string(),
                    ])),
                    Some(path) if path == unavailable_socket => {
                        Err(anyhow::anyhow!("tmux server is unavailable"))
                    }
                    Some(_) => Ok(HashSet::new()),
                }
            });
        let signals = RefCell::new(Vec::new());

        let cleaned = cleanup_sessions(
            &sessions,
            SessionCleanupContext {
                worktree_path: &worktree_path,
                alive_panes_by_socket: &alive_panes_by_socket,
                own_codex_session_id: None,
            },
            |socket_path, pane_id| {
                signals
                    .borrow_mut()
                    .push((socket_path.map(str::to_string), pane_id.to_string()));
            },
            |_| Ok(true),
            |_| PendingCardCleanup {
                _todo_resolution_lock: None,
                error: None,
            },
            |_| Ok(()),
            |_| Ok(()),
        );

        assert_eq!(
            (cleaned, queries.into_inner(), signals.into_inner(),),
            (
                4,
                vec![
                    None,
                    Some(private_socket.to_string()),
                    Some(unavailable_socket.to_string()),
                ],
                vec![
                    (None, "%legacy".to_string()),
                    (Some(private_socket.to_string()), "%private-a".to_string()),
                    (Some(private_socket.to_string()), "%private-b".to_string()),
                ],
            ),
        );
    }

    #[rstest]
    fn defers_current_codex_archive_until_cleanup_finishes() {
        let worktree_path = PathBuf::from("/tmp/example-worktree");
        let sessions = vec![
            make_session("current-session", &worktree_path, Engine::Codex),
            make_session("other-session", &worktree_path, Engine::Codex),
        ];
        let alive_panes_by_socket = tmux::PaneLiveness::default();
        let events = RefCell::new(Vec::new());

        let cleaned = cleanup_sessions_with_deferred_archive(
            &sessions,
            SessionCleanupContext {
                worktree_path: &worktree_path,
                alive_panes_by_socket: &alive_panes_by_socket,
                own_codex_session_id: Some("current-session"),
            },
            SessionCleanupOperations {
                send_sigterm: |_socket_path: Option<&str>, _pane_id: &str| {},
                delete_session: |session_id: &str| {
                    events.borrow_mut().push(format!("delete:{session_id}"));
                    Ok(true)
                },
                withdraw_pending_cards: |_session_id: &str| PendingCardCleanup {
                    _todo_resolution_lock: None,
                    error: None,
                },
                remove_notification_group: |session_id: &str| {
                    events
                        .borrow_mut()
                        .push(format!("notification:{session_id}"));
                    Ok(())
                },
                archive_codex_thread: |session_id: &str| {
                    events.borrow_mut().push(format!("archive:{session_id}"));
                    Ok(())
                },
                defer_archive: |session_id: &str| {
                    events.borrow_mut().push(format!("defer:{session_id}"));
                    Ok(())
                },
            },
        );

        assert_eq!(
            (cleaned, events.into_inner()),
            (
                2,
                vec![
                    "delete:current-session".to_string(),
                    "notification:current-session".to_string(),
                    "archive:other-session".to_string(),
                    "delete:other-session".to_string(),
                    "notification:other-session".to_string(),
                    "defer:current-session".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[case::running(SessionStatus::Running, true)]
    #[case::waiting(SessionStatus::WaitingInput, true)]
    #[case::stopped(SessionStatus::Stopped, true)]
    #[case::failed(SessionStatus::Failed, true)]
    #[case::paused(SessionStatus::Paused, false)]
    #[case::ended(SessionStatus::Ended, false)]
    fn should_sigterm_session_by_status(#[case] status: SessionStatus, #[case] expected: bool) {
        // Paused sessions were already SIGTERM'd by `agent sweep` so the pane
        // now hosts the user's shell -- signaling it would kill the caller
        // when `a agent close` runs from that same pane.
        assert_eq!(should_sigterm_session(status), expected);
    }
}
