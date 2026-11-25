//! `a agent close` shuts down a tracked agent session and removes its tmux pane.

use std::io;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;

mod child;
mod safety;
mod session_close;
mod target;
mod worktree;
use target::{close_default_target, close_target, resolve_default_target};
pub(crate) use worktree::close_worktree_path;
use worktree::linked_worktree_root;
#[cfg(test)]
mod test_support;

use safety::{check_draft_state, ensure_session_is_idle, write_draft_unavailable_warning};

use super::auto_pause;
use super::graceful_quit::GracefulQuitRequester;
use super::resume;
use super::signal::{LibcSignalSender, SignalSender};
use super::types::{Engine, Session, SessionStatus, TmuxInfo};
use crate::infra::process::{self, ProcessSnapshot};
use crate::infra::tmux;
use crate::shared::config::load_config_or_default;
use crate::shared::env_var::EnvVars;
use crate::shared::worktree_delete;

const MAX_DESCENDANT_NODES: usize = 64;
const SIGTERM_GRACE_PERIOD: Duration = Duration::from_secs(5);
const DEFAULT_AUTO_PAUSE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub(crate) async fn close_from_watch_web(
    session: &Session,
    keep_worktree: bool,
    stop_and_close: bool,
) -> Result<()> {
    session_close::close_session_value_from_watch_web(session, keep_worktree, stop_and_close).await
}

#[derive(Args, Clone, Default, PartialEq, Eq)]
pub struct CloseArgs {
    /// Session ID, worktree name, or worktree path to close.
    /// Defaults to the session in the current pane or the current worktree.
    pub target: Option<String>,

    /// Refuse to close the target unless it is a direct child session.
    #[arg(long, requires = "target")]
    pub child: bool,

    /// Override session safety checks and unmerged-worktree confirmation; delete worktrees with non-shell processes or uncommitted changes.
    #[arg(long)]
    pub force: bool,

    /// Skip worktree deletion hooks.
    #[arg(long, conflicts_with = "keep_worktree")]
    pub skip_hooks: bool,

    /// Keep the linked worktree and branch when closing a session.
    #[arg(long, conflicts_with = "skip_hooks")]
    pub keep_worktree: bool,
}

pub async fn run(args: &CloseArgs) -> Result<()> {
    let child_parent_session_id = child::parent_session_id(args)?;

    if let Some(target) = args.target.as_deref() {
        return close_target(target, args, child_parent_session_id.as_deref()).await;
    }

    let current_worktree_root = || {
        let cwd = std::env::current_dir().context("Failed to get current directory")?;
        Ok(linked_worktree_root(&cwd))
    };
    let target = resolve_default_target(
        resume::resolve_session_id_from_pane(),
        super::store::load_session,
        current_worktree_root,
    )?;
    close_default_target(target, args).await
}

fn close_tracked_session(session: &Session, force: bool) -> Result<()> {
    close_tracked_session_with_runtime(session, force, &LiveCloseRuntime)
}

fn close_tracked_session_with_runtime<R: CloseRuntime>(
    session: &Session,
    force: bool,
    runtime: &R,
) -> Result<()> {
    close_tracked_session_with_runtime_and_stop(session, force, false, runtime)
}

fn close_tracked_session_with_runtime_and_stop<R: CloseRuntime>(
    session: &Session,
    force: bool,
    stop_and_close: bool,
    runtime: &R,
) -> Result<()> {
    let mut session = session.clone();
    super::session_status::include_pending_status(&mut session);
    if stop_and_close {
        close_session_with_stop_and_close(&session, force, true, runtime)?;
    } else {
        close_session(&session, force, runtime)?;
    }
    if session.tmux_info.is_none() {
        let ended = runtime
            .mark_session_ended_if_unchanged(&session)
            .map_err(SessionLifecyclePersistenceError::new)?;
        if !ended {
            bail!(
                "Agent session `{}` changed while it was being closed; retry this command",
                session.session_id
            );
        }
    } else {
        runtime
            .mark_session_ended(&session.session_id)
            .map_err(SessionLifecyclePersistenceError::new)?;
    }
    Ok(())
}

pub(crate) fn resolve_agent_pid_for_session(session: &Session) -> Result<Option<u32>> {
    let Some(tmux_info) = session.tmux_info.as_ref() else {
        return Ok(None);
    };
    resolve_agent_pid_for_pane(tmux_info, session.engine)
}

fn resolve_agent_pid_for_pane(tmux_info: &TmuxInfo, engine: Engine) -> Result<Option<u32>> {
    let Some(pane_pid) =
        tmux::get_pane_pid_for_socket(tmux_info.socket_path.as_deref(), &tmux_info.pane_id)?
    else {
        return Ok(None);
    };
    let snapshot = ProcessSnapshot::capture().context("Could not inspect running processes")?;
    Ok(snapshot.find_self_or_descendant_by_command(
        pane_pid,
        engine.process_name(),
        MAX_DESCENDANT_NODES,
    ))
}

#[derive(Debug, thiserror::Error)]
#[error(
    "the session closed, but its lifecycle storage cleanup failed; retry Close when storage is available: {0}"
)]
pub(crate) struct SessionLifecyclePersistenceError(#[source] anyhow::Error);

impl SessionLifecyclePersistenceError {
    pub(crate) fn new(source: anyhow::Error) -> Self {
        Self(source)
    }
}

trait CloseRuntime {
    fn pane_exists(&self, tmux_info: &TmuxInfo) -> Result<bool>;
    fn pane_session_id(&self, tmux_info: &TmuxInfo) -> Option<String>;
    fn has_draft(&self, tmux_info: &TmuxInfo, engine: Engine) -> Option<bool>;
    fn latest_codex_turn_status(&self, thread_id: &str) -> Result<Option<String>>;
    fn resolve_agent_pid(&self, tmux_info: &TmuxInfo, engine: Engine) -> Result<Option<u32>>;
    fn request_graceful_quit(&self, tmux_info: &TmuxInfo, pid: u32) -> io::Result<bool>;
    fn send_sigterm(&self, pid: u32) -> io::Result<()>;
    fn wait_for_exit(&self, pid: u32) -> bool;
    fn end_codex_session(&self, session_id: &str) -> Result<()>;
    fn mark_session_ended(&self, session_id: &str) -> Result<()>;
    fn mark_session_ended_if_unchanged(&self, expected_session: &Session) -> Result<bool>;
    fn kill_pane(&self, tmux_info: &TmuxInfo) -> Result<()>;

    fn unknown_draft_timeout(&self) -> Duration {
        DEFAULT_AUTO_PAUSE_TIMEOUT
    }
}

struct LiveCloseRuntime;

impl CloseRuntime for LiveCloseRuntime {
    fn pane_exists(&self, tmux_info: &TmuxInfo) -> Result<bool> {
        Ok(
            tmux::list_all_pane_ids_for_socket(tmux_info.socket_path.as_deref())?
                .contains(&tmux_info.pane_id),
        )
    }

    fn pane_session_id(&self, tmux_info: &TmuxInfo) -> Option<String> {
        super::types::resolve_session_option(|option| {
            tmux::get_pane_option_for_socket(
                tmux_info.socket_path.as_deref(),
                &tmux_info.pane_id,
                option,
            )
        })
    }

    fn has_draft(&self, tmux_info: &TmuxInfo, engine: Engine) -> Option<bool> {
        super::pane::input::pane_has_draft_for_socket(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
            engine,
        )
    }

    fn latest_codex_turn_status(&self, thread_id: &str) -> Result<Option<String>> {
        let mut client = super::codex_steer::Client::connect()?;
        client.latest_turn_status(thread_id)
    }

    fn resolve_agent_pid(&self, tmux_info: &TmuxInfo, engine: Engine) -> Result<Option<u32>> {
        resolve_agent_pid_for_pane(tmux_info, engine)
    }

    fn request_graceful_quit(&self, tmux_info: &TmuxInfo, pid: u32) -> io::Result<bool> {
        LibcSignalSender.request_and_wait(tmux_info.socket_path.as_deref(), &tmux_info.pane_id, pid)
    }

    fn send_sigterm(&self, pid: u32) -> io::Result<()> {
        LibcSignalSender.send(pid, libc::SIGTERM)
    }

    fn wait_for_exit(&self, pid: u32) -> bool {
        process::wait_for_process_exit(pid, SIGTERM_GRACE_PERIOD)
    }

    fn end_codex_session(&self, session_id: &str) -> Result<()> {
        super::hook::end_codex_session(session_id)
    }

    fn mark_session_ended(&self, session_id: &str) -> Result<()> {
        super::hook::mark_session_ended_after_close(session_id)
    }

    fn mark_session_ended_if_unchanged(&self, expected_session: &Session) -> Result<bool> {
        super::hook::mark_session_ended_after_close_if_unchanged(expected_session)
    }

    fn kill_pane(&self, tmux_info: &TmuxInfo) -> Result<()> {
        Ok(tmux::kill_pane_for_socket(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
        )?)
    }

    fn unknown_draft_timeout(&self) -> Duration {
        let config = load_config_or_default();
        match auto_pause::parse_duration(&config.agent.auto_pause.timeout) {
            Ok(timeout) => timeout,
            Err(error) => {
                tracing::warn!(
                    event = "agent.close.auto_pause_timeout_invalid",
                    error = %error,
                    "Using the default timeout when draft state is unavailable"
                );
                DEFAULT_AUTO_PAUSE_TIMEOUT
            }
        }
    }
}

struct SessionClosePlan<'a> {
    tmux_info: Option<&'a TmuxInfo>,
    agent_pid: Option<u32>,
    should_end_codex_session: bool,
    draft_inspection: DraftInspection,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DraftInspection {
    NotChecked,
    NoDraft,
    Unavailable,
}

fn prepare_close_session<'a, R: CloseRuntime>(
    session: &'a Session,
    force: bool,
    stop_and_close: bool,
    runtime: &R,
) -> Result<Option<SessionClosePlan<'a>>> {
    let Some(tmux_info) = session.tmux_info.as_ref() else {
        let draft_inspection = if !force
            && !stop_and_close
            && session.status != SessionStatus::Paused
        {
            safety::validate_session_without_tmux_info(session, runtime.unknown_draft_timeout())?
        } else {
            DraftInspection::NotChecked
        };
        return Ok(Some(SessionClosePlan {
            tmux_info: None,
            agent_pid: None,
            should_end_codex_session: false,
            draft_inspection,
        }));
    };
    if !runtime.pane_exists(tmux_info)? {
        return Ok(None);
    }
    ensure_pane_session(runtime, tmux_info, &session.session_id)?;

    if !force && !stop_and_close {
        let effective_status = if session.engine == Engine::Codex
            && session.status == SessionStatus::Running
            && match runtime.latest_codex_turn_status(&session.session_id) {
                Ok(Some(status)) => status == "interrupted",
                Ok(None) => false,
                Err(error) => {
                    tracing::warn!(
                        event = "agent.close.codex_turn_read_failed",
                        session = %session.session_id,
                        error = %error,
                    );
                    false
                }
            } {
            session.status_after_interrupted_codex_turn()
        } else {
            session.status
        };
        ensure_session_is_idle(session, effective_status)?;
    }

    let (agent_pid, should_end_codex_session, draft_inspection) = match runtime
        .resolve_agent_pid(tmux_info, session.engine)?
    {
        Some(pid) => {
            let draft_inspection = if force {
                DraftInspection::NotChecked
            } else {
                check_draft_state(runtime, tmux_info, session)?
            };
            (Some(pid), true, draft_inspection)
        }
        None if force => (None, true, DraftInspection::NotChecked),
        None if !matches!(session.status, SessionStatus::Ended | SessionStatus::Paused) => {
            bail!(
                "Could not find a running agent process for session `{}`; its pane was left open",
                session.session_id
            );
        }
        None => (None, true, DraftInspection::NotChecked),
    };

    Ok(Some(SessionClosePlan {
        tmux_info: Some(tmux_info),
        agent_pid,
        should_end_codex_session,
        draft_inspection,
    }))
}

fn close_session<R: CloseRuntime>(session: &Session, force: bool, runtime: &R) -> Result<()> {
    let stderr = io::stderr();
    close_session_with_warning_writer(session, force, runtime, &mut stderr.lock())
}

fn close_session_with_stop_and_close<R: CloseRuntime>(
    session: &Session,
    force: bool,
    stop_and_close: bool,
    runtime: &R,
) -> Result<()> {
    let stderr = io::stderr();
    close_session_with_stop_and_close_warning_writer(
        session,
        force,
        stop_and_close,
        runtime,
        &mut stderr.lock(),
    )
}

fn close_session_with_warning_writer<R: CloseRuntime>(
    session: &Session,
    force: bool,
    runtime: &R,
    warning_writer: &mut impl io::Write,
) -> Result<()> {
    close_session_with_stop_and_close_warning_writer(session, force, false, runtime, warning_writer)
}

fn close_session_with_stop_and_close_warning_writer<R: CloseRuntime>(
    session: &Session,
    force: bool,
    stop_and_close: bool,
    runtime: &R,
    warning_writer: &mut impl io::Write,
) -> Result<()> {
    let Some(plan) = prepare_close_session(session, force, stop_and_close, runtime)? else {
        return Ok(());
    };
    if plan.draft_inspection == DraftInspection::Unavailable
        && let Err(error) = write_draft_unavailable_warning(warning_writer, &session.session_id)
    {
        tracing::warn!(
            event = "agent.close.draft_state_warning_failed",
            session = %session.session_id,
            error = %error,
        );
    }

    let Some(tmux_info) = plan.tmux_info else {
        return Ok(());
    };
    let pane_id = tmux_info.pane_id.as_str();

    if let Some(pid) = plan.agent_pid {
        let exited = match runtime.request_graceful_quit(tmux_info, pid) {
            Ok(true) => true,
            Ok(false) => {
                tracing::warn!(
                    event = "agent.close.ctrl_d_timeout",
                    session = %session.session_id,
                    pane_id,
                    pid,
                );
                false
            }
            Err(error) => {
                tracing::warn!(
                    event = "agent.close.ctrl_d_failed",
                    session = %session.session_id,
                    pane_id,
                    error = %error,
                );
                false
            }
        };

        if !exited {
            match runtime.send_sigterm(pid) {
                Ok(()) => {}
                Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "Could not send SIGTERM to agent session `{}`",
                            session.session_id
                        )
                    });
                }
            }
            if !runtime.wait_for_exit(pid) {
                bail!(
                    "Agent session `{}` did not exit after SIGTERM; its pane was left open",
                    session.session_id
                );
            }
        }
    }

    if runtime.pane_exists(tmux_info)? {
        ensure_pane_session(runtime, tmux_info, &session.session_id)?;
        if plan.should_end_codex_session
            && session.engine == Engine::Codex
            && session.status != SessionStatus::Ended
        {
            runtime.end_codex_session(&session.session_id)?;
        }
        runtime.kill_pane(tmux_info)?;
    }
    Ok(())
}

fn ensure_pane_session<R: CloseRuntime>(
    runtime: &R,
    tmux_info: &TmuxInfo,
    session_id: &str,
) -> Result<()> {
    if runtime.pane_session_id(tmux_info).as_deref() != Some(session_id) {
        bail!(
            "Pane `{}` is no longer bound to agent session `{session_id}`",
            tmux_info.pane_id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;

    use chrono::Utc;
    use indoc::indoc;
    use rstest::rstest;

    use super::test_support::{
        Call, FakeRuntime, draft_call, graceful_quit_call, kill_pane_call, pane_exists_call,
        pane_session_call, resolve_pid_call, session,
    };
    use super::*;

    struct PanelessCloseCase {
        status: SessionStatus,
        force: bool,
        pending_task: bool,
        recent_activity: bool,
        stop_and_close: bool,
        expected_error: Option<&'static str>,
        expected_ended_sessions: Vec<&'static str>,
    }

    #[rstest]
    #[case::stopped(
        SessionStatus::Stopped,
        Ok(()),
        vec!["session-1"],
    )]
    #[case::running(
        SessionStatus::Running,
        Err("Agent session `session-1` is running or waiting for input; complete any requested input and wait for the current turn to finish, then retry this command"),
        vec![],
    )]
    fn close_marks_session_ended_only_after_shutdown_succeeds(
        mut session: Session,
        #[case] status: SessionStatus,
        #[case] expected_result: std::result::Result<(), &str>,
        #[case] expected_ended_sessions: Vec<&str>,
    ) {
        session.status = status;
        let runtime = FakeRuntime::default();

        let result = close_tracked_session_with_runtime(&session, false, &runtime);

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                runtime.ended_sessions.into_inner(),
            ),
            (
                expected_result.map_err(str::to_string),
                expected_ended_sessions
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            ),
        );
    }

    #[rstest]
    #[case::idle_session_is_ended(
        PanelessCloseCase {
            status: SessionStatus::Stopped,
            force: false,
            pending_task: false,
            recent_activity: false,
            stop_and_close: false,
            expected_error: None,
            expected_ended_sessions: vec!["session-1"],
        },
    )]
    #[case::active_session_is_rejected(
        PanelessCloseCase {
            status: SessionStatus::Running,
            force: false,
            pending_task: false,
            recent_activity: false,
            stop_and_close: false,
            expected_error: Some("Agent session `session-1` is running or waiting for input; complete any requested input and wait for the current turn to finish, then retry this command"),
            expected_ended_sessions: vec![],
        },
    )]
    #[case::pending_tasks_are_rejected(
        PanelessCloseCase {
            status: SessionStatus::Stopped,
            force: false,
            pending_task: true,
            recent_activity: false,
            stop_and_close: false,
            expected_error: Some(indoc! {"
                Agent session `session-1` has pending background or agent tasks:
                  Background task ID: `task-1`
                Finish the pending work, then retry this command.
            "}.trim_end()),
            expected_ended_sessions: vec![],
        },
    )]
    #[case::recent_activity_is_rejected(
        PanelessCloseCase {
            status: SessionStatus::Stopped,
            force: false,
            pending_task: false,
            recent_activity: true,
            stop_and_close: false,
            expected_error: Some("Could not check for an unsent draft in agent session `session-1`; recent activity prevents closing it, pass --force to close it"),
            expected_ended_sessions: vec![],
        },
    )]
    #[case::forced_close_skips_safety_checks(
        PanelessCloseCase {
            status: SessionStatus::Running,
            force: true,
            pending_task: true,
            recent_activity: true,
            stop_and_close: false,
            expected_error: None,
            expected_ended_sessions: vec!["session-1"],
        },
    )]
    #[case::paused_close_skips_safety_checks(
        PanelessCloseCase {
            status: SessionStatus::Paused,
            force: false,
            pending_task: false,
            recent_activity: true,
            stop_and_close: false,
            expected_error: None,
            expected_ended_sessions: vec!["session-1"],
        },
    )]
    #[case::stop_and_close_skips_paneless_safety_checks(
        PanelessCloseCase {
            status: SessionStatus::Running,
            force: false,
            pending_task: true,
            recent_activity: true,
            stop_and_close: true,
            expected_error: None,
            expected_ended_sessions: vec!["session-1"],
        },
    )]
    fn close_handles_sessions_without_tmux_panes(
        mut session: Session,
        #[case] case: PanelessCloseCase,
    ) {
        session.tmux_info = None;
        session.status = case.status;
        session.updated_at = if case.recent_activity {
            Utc::now()
        } else {
            Utc::now() - chrono::Duration::hours(1)
        };
        if case.pending_task {
            session.pending_bg_task_ids.insert("task-1".to_string());
        }
        let runtime = FakeRuntime::default();

        let result = close_tracked_session_with_runtime_and_stop(
            &session,
            case.force,
            case.stop_and_close,
            &runtime,
        );

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                runtime.ended_sessions.into_inner(),
                runtime.calls.into_inner(),
            ),
            (
                case.expected_error
                    .map_or(Ok(()), |error| Err(error.to_string())),
                case.expected_ended_sessions
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                vec![],
            ),
        );
    }

    #[rstest]
    #[case::running(SessionStatus::Running, false, false)]
    #[case::waiting_for_input(SessionStatus::WaitingInput, false, false)]
    #[case::pending_background_task(SessionStatus::Stopped, true, false)]
    #[case::pending_agent_task(SessionStatus::Stopped, false, true)]
    fn close_rejects_active_or_pending_sessions(
        mut session: Session,
        #[case] status: SessionStatus,
        #[case] pending_bg: bool,
        #[case] pending_agent: bool,
    ) {
        session.status = status;
        if pending_bg {
            session.pending_bg_task_ids.insert("task-1".to_string());
        }
        if pending_agent {
            session
                .pending_agent_task_ids
                .insert("agent-task-1".to_string());
        }
        let runtime = FakeRuntime::default();

        let result = close_session(&session, false, &runtime);

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            (
                if status == SessionStatus::Running || status == SessionStatus::WaitingInput {
                    "Agent session `session-1` is running or waiting for input; complete any requested input and wait for the current turn to finish, then retry this command"
                } else if pending_bg {
                    indoc! {"
                        Agent session `session-1` has pending background or agent tasks:
                          Background task ID: `task-1`
                        Finish the pending work, then retry this command.
                    "}.trim_end()
                } else {
                    indoc! {"
                        Agent session `session-1` has pending background or agent tasks:
                          Agent task ID: `agent-task-1`
                        Finish the pending work, then retry this command.
                    "}.trim_end()
                }
                .to_string(),
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                ],
            ),
        );
    }

    #[test]
    fn confirmed_stop_and_close_shuts_down_active_session() {
        let mut session = session();
        session.status = SessionStatus::Running;
        session.pending_bg_task_ids.insert("task-1".to_string());
        session
            .pending_permission_agent_ids
            .insert("fixture-agent-id".to_string());
        let runtime = FakeRuntime::default();

        let result = close_tracked_session_with_runtime_and_stop(&session, false, true, &runtime);

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                runtime.calls.into_inner(),
                runtime.ended_sessions.into_inner(),
            ),
            (
                Ok(()),
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    resolve_pid_call(Engine::Claude),
                    draft_call(Engine::Claude),
                    graceful_quit_call(42),
                    pane_exists_call(),
                    pane_session_call(),
                    kill_pane_call(),
                ],
                vec!["session-1".to_string()],
            ),
        );
    }

    #[test]
    fn confirmed_stop_and_close_still_protects_an_unsent_draft() {
        let mut session = session();
        session.status = SessionStatus::Running;
        let runtime = FakeRuntime {
            draft: Some(true),
            ..FakeRuntime::default()
        };

        let result = close_tracked_session_with_runtime_and_stop(&session, false, true, &runtime);

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                runtime.calls.into_inner(),
                runtime.ended_sessions.into_inner(),
            ),
            (
                Err(
                    "Agent session `session-1` has an unsent draft; pass --force to close it"
                        .to_string()
                ),
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    resolve_pid_call(Engine::Claude),
                    draft_call(Engine::Claude),
                ],
                vec![],
            ),
        );
    }

    #[test]
    fn close_allows_a_codex_turn_interrupted_by_the_user() {
        let mut session = session();
        session.engine = Engine::Codex;
        session.status = SessionStatus::Running;
        let runtime = FakeRuntime {
            turn_status: Ok(Some("interrupted".to_string())),
            ..FakeRuntime::default()
        };

        let result = close_session(&session, false, &runtime);

        assert_eq!(
            (result.is_ok(), runtime.calls.into_inner()),
            (
                true,
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    Call::LatestTurnStatus("session-1".to_string()),
                    resolve_pid_call(Engine::Codex),
                    draft_call(Engine::Codex),
                    graceful_quit_call(42),
                    pane_exists_call(),
                    pane_session_call(),
                    Call::EndCodexSession("session-1".to_string()),
                    kill_pane_call(),
                ],
            ),
        );
    }

    #[rstest]
    #[case::no_latest_turn(Ok(None))]
    #[case::turn_in_progress(Ok(Some("inProgress".to_string())))]
    #[case::app_server_unavailable(Err("fixture app-server unavailable".to_string()))]
    fn close_keeps_running_codex_sessions_active_without_interrupted_status(
        mut session: Session,
        #[case] turn_status: std::result::Result<Option<String>, String>,
    ) {
        session.engine = Engine::Codex;
        session.status = SessionStatus::Running;
        let runtime = FakeRuntime {
            turn_status,
            ..FakeRuntime::default()
        };

        let result = close_session(&session, false, &runtime);

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            (
                "Agent session `session-1` is running or waiting for input; complete any requested input and wait for the current turn to finish, then retry this command"
                    .to_string(),
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    Call::LatestTurnStatus("session-1".to_string()),
                ],
            ),
        );
    }

    #[test]
    fn close_keeps_rejecting_pending_tasks_after_an_interrupted_codex_turn() {
        let mut session = session();
        session.engine = Engine::Codex;
        session.status = SessionStatus::Running;
        session.pending_bg_task_ids.insert("task-1".to_string());
        let runtime = FakeRuntime {
            turn_status: Ok(Some("interrupted".to_string())),
            ..FakeRuntime::default()
        };

        let result = close_session(&session, false, &runtime);

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            (
                indoc! {"
                    Agent session `session-1` has pending background or agent tasks:
                      Background task ID: `task-1`
                    Finish the pending work, then retry this command.
                "}
                .trim_end()
                .to_string(),
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    Call::LatestTurnStatus("session-1".to_string()),
                ],
            ),
        );
    }

    #[test]
    fn close_keeps_rejecting_pending_permissions_after_an_interrupted_codex_turn() {
        let mut session = session();
        session.engine = Engine::Codex;
        session.status = SessionStatus::Running;
        session
            .pending_permission_agent_ids
            .insert("fixture-agent-id".to_string());
        let runtime = FakeRuntime {
            turn_status: Ok(Some("interrupted".to_string())),
            ..FakeRuntime::default()
        };

        let result = close_session(&session, false, &runtime);

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            (
                "Agent session `session-1` is running or waiting for input; complete any requested input and wait for the current turn to finish, then retry this command"
                    .to_string(),
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    Call::LatestTurnStatus("session-1".to_string()),
                ],
            ),
        );
    }

    #[test]
    fn close_rejects_pending_permissions_when_the_session_is_stopped() {
        let mut session = session();
        session.status = SessionStatus::Stopped;
        session
            .pending_permission_agent_ids
            .insert("fixture-agent-id".to_string());
        let runtime = FakeRuntime::default();

        let result = close_session(&session, false, &runtime);

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            (
                "Agent session `session-1` has pending permission requests; resolve them and retry this command"
                    .to_string(),
                vec![pane_exists_call(), pane_session_call()],
            ),
        );
    }

    #[rstest]
    #[case::claude_draft_present(
        Engine::Claude,
        Some(true),
        "Agent session `session-1` has an unsent draft; pass --force to close it"
    )]
    #[case::codex_draft_present(
        Engine::Codex,
        Some(true),
        "Agent session `session-1` has an unsent draft; pass --force to close it"
    )]
    #[case::claude_draft_unknown(
        Engine::Claude,
        None,
        "Could not check for an unsent draft in agent session `session-1`; recent activity prevents closing it, pass --force to close it"
    )]
    #[case::codex_draft_unknown(
        Engine::Codex,
        None,
        "Could not check for an unsent draft in agent session `session-1`; recent activity prevents closing it, pass --force to close it"
    )]
    fn close_rejects_unsent_or_uninspectable_drafts(
        mut session: Session,
        #[case] engine: Engine,
        #[case] draft: Option<bool>,
        #[case] expected_error: &str,
    ) {
        session.engine = engine;
        let runtime = FakeRuntime {
            draft,
            ..FakeRuntime::default()
        };

        let result = close_session(&session, false, &runtime);

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            (
                expected_error.to_string(),
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    resolve_pid_call(engine),
                    draft_call(engine),
                ],
            ),
        );
    }

    #[rstest]
    #[case::claude_stopped(Engine::Claude, SessionStatus::Stopped)]
    #[case::claude_paused(Engine::Claude, SessionStatus::Paused)]
    #[case::claude_ended(Engine::Claude, SessionStatus::Ended)]
    #[case::codex_stopped(Engine::Codex, SessionStatus::Stopped)]
    fn close_allows_unknown_draft_when_session_update_is_stale(
        mut session: Session,
        #[case] engine: Engine,
        #[case] status: SessionStatus,
    ) {
        session.engine = engine;
        session.status = status;
        session.updated_at = Utc::now() - chrono::TimeDelta::hours(1);
        let runtime = FakeRuntime {
            draft: None,
            ..FakeRuntime::default()
        };

        let result = close_session(&session, false, &runtime);
        let mut expected_calls = vec![
            pane_exists_call(),
            pane_session_call(),
            resolve_pid_call(engine),
            draft_call(engine),
            graceful_quit_call(42),
            pane_exists_call(),
            pane_session_call(),
        ];
        if engine == Engine::Codex {
            expected_calls.push(Call::EndCodexSession("session-1".to_string()));
        }
        expected_calls.push(kill_pane_call());

        assert_eq!(
            (result.is_ok(), runtime.calls.into_inner()),
            (true, expected_calls),
        );
    }

    #[rstest]
    #[case::paused(SessionStatus::Paused)]
    #[case::ended(SessionStatus::Ended)]
    fn close_blocks_unknown_draft_for_recent_terminal_sessions(
        mut session: Session,
        #[case] status: SessionStatus,
    ) {
        session.status = status;
        session.updated_at = Utc::now();
        let runtime = FakeRuntime {
            draft: None,
            ..FakeRuntime::default()
        };

        let result = close_session(&session, false, &runtime);

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            (
                "Could not check for an unsent draft in agent session `session-1`; recent activity prevents closing it, pass --force to close it"
                    .to_string(),
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    resolve_pid_call(Engine::Claude),
                    draft_call(Engine::Claude),
                ],
            ),
        );
    }

    #[test]
    fn close_warns_when_draft_state_is_unavailable() {
        let mut session = session();
        session.updated_at = Utc::now() - chrono::TimeDelta::hours(1);
        let runtime = FakeRuntime {
            draft: None,
            ..FakeRuntime::default()
        };
        let mut stderr = Vec::new();

        let result = close_session_with_warning_writer(&session, false, &runtime, &mut stderr);

        assert_eq!(
            (result.is_ok(), stderr, runtime.calls.into_inner()),
            (
                true,
                b"Warning: could not inspect the unsent draft for session `session-1`; using its last update time for the close safety check\n"
                    .to_vec(),
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    resolve_pid_call(Engine::Claude),
                    draft_call(Engine::Claude),
                    graceful_quit_call(42),
                    pane_exists_call(),
                    pane_session_call(),
                    kill_pane_call(),
                ],
            ),
        );
    }

    #[test]
    fn paneless_close_warns_when_draft_state_is_unavailable() {
        let mut session = session();
        session.tmux_info = None;
        session.updated_at = Utc::now() - chrono::TimeDelta::hours(1);
        let runtime = FakeRuntime::default();
        let mut stderr = Vec::new();

        let result = close_session_with_warning_writer(&session, false, &runtime, &mut stderr);

        assert_eq!(
            (result.map_err(|error| error.to_string()), stderr, runtime.calls.into_inner()),
            (
                Ok(()),
                b"Warning: could not inspect the unsent draft for session `session-1`; using its last update time for the close safety check\n"
                    .to_vec(),
                vec![],
            ),
        );
    }

    #[test]
    fn close_sends_ctrl_d_then_kills_only_the_target_pane() {
        let runtime = FakeRuntime::default();

        let result = close_session(&session(), false, &runtime);

        assert_eq!(
            (result.is_ok(), runtime.calls.into_inner()),
            (
                true,
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    resolve_pid_call(Engine::Claude),
                    draft_call(Engine::Claude),
                    graceful_quit_call(42),
                    pane_exists_call(),
                    pane_session_call(),
                    kill_pane_call(),
                ],
            ),
        );
    }

    #[test]
    fn close_ends_codex_after_exit_and_before_killing_the_pane() {
        let mut session = session();
        session.engine = Engine::Codex;
        let runtime = FakeRuntime::default();

        let result = close_session(&session, true, &runtime);

        assert_eq!(
            (result.is_ok(), runtime.calls.into_inner()),
            (
                true,
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    resolve_pid_call(Engine::Codex),
                    graceful_quit_call(42),
                    pane_exists_call(),
                    pane_session_call(),
                    Call::EndCodexSession("session-1".to_string()),
                    kill_pane_call(),
                ],
            ),
        );
    }

    #[test]
    fn close_fails_when_tmux_cannot_confirm_whether_the_pane_exists() {
        let runtime = FakeRuntime {
            pane_exists: Err(anyhow::anyhow!("tmux is unavailable")),
            ..FakeRuntime::default()
        };

        let result = close_session(&session(), false, &runtime);

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            ("tmux is unavailable".to_string(), vec![pane_exists_call()],),
        );
    }

    #[test]
    fn close_stops_when_tmux_cannot_resolve_the_pane_pid() {
        let runtime = FakeRuntime {
            pid_error: Some("tmux is unavailable".to_string()),
            ..FakeRuntime::default()
        };

        let result = close_session(&session(), false, &runtime);

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            (
                "tmux is unavailable".to_string(),
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    resolve_pid_call(Engine::Claude),
                ],
            ),
        );
    }

    #[test]
    fn close_uses_the_default_socket_for_legacy_session_info() {
        let mut session = session();
        if let Some(tmux_info) = session.tmux_info.as_mut() {
            tmux_info.socket_path = None;
        }
        let runtime = FakeRuntime {
            pane_exists: Ok(false),
            ..FakeRuntime::default()
        };

        let result = close_session(&session, false, &runtime);

        assert_eq!(
            (result.is_ok(), runtime.calls.into_inner()),
            (true, vec![Call::PaneExists(None, "%42".to_string())]),
        );
    }

    #[test]
    fn close_succeeds_without_action_when_the_pane_is_gone() {
        let runtime = FakeRuntime {
            pane_exists: Ok(false),
            ..FakeRuntime::default()
        };

        let result = close_session(&session(), false, &runtime);

        assert_eq!(
            (result.is_ok(), runtime.calls.into_inner()),
            (true, vec![pane_exists_call()]),
        );
    }

    #[test]
    fn close_falls_back_to_sigterm_when_ctrl_d_times_out() {
        let runtime = FakeRuntime {
            graceful_result: Ok(false),
            ..FakeRuntime::default()
        };

        let result = close_session(&session(), false, &runtime);

        assert_eq!(
            (result.is_ok(), runtime.calls.into_inner()),
            (
                true,
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    resolve_pid_call(Engine::Claude),
                    draft_call(Engine::Claude),
                    graceful_quit_call(42),
                    Call::Sigterm(42),
                    Call::WaitForExit(42),
                    pane_exists_call(),
                    pane_session_call(),
                    kill_pane_call(),
                ],
            ),
        );
    }

    #[test]
    fn close_falls_back_to_sigterm_when_ctrl_d_fails() {
        let runtime = FakeRuntime {
            graceful_result: Err(io::Error::other("tmux send-keys failed")),
            ..FakeRuntime::default()
        };

        let result = close_session(&session(), false, &runtime);

        assert_eq!(
            (result.is_ok(), runtime.calls.into_inner()),
            (
                true,
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    resolve_pid_call(Engine::Claude),
                    draft_call(Engine::Claude),
                    graceful_quit_call(42),
                    Call::Sigterm(42),
                    Call::WaitForExit(42),
                    pane_exists_call(),
                    pane_session_call(),
                    kill_pane_call(),
                ],
            ),
        );
    }

    #[test]
    fn close_force_bypasses_safety_checks() {
        let mut session = session();
        session.status = SessionStatus::Running;
        session.pending_bg_task_ids.insert("task-1".to_string());
        let runtime = FakeRuntime {
            draft: Some(true),
            ..FakeRuntime::default()
        };

        let result = close_session(&session, true, &runtime);

        assert_eq!(
            (result.is_ok(), runtime.calls.into_inner()),
            (
                true,
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    resolve_pid_call(Engine::Claude),
                    graceful_quit_call(42),
                    pane_exists_call(),
                    pane_session_call(),
                    kill_pane_call(),
                ],
            ),
        );
    }

    #[rstest]
    #[case::normal(false)]
    #[case::forced(true)]
    fn close_refuses_to_kill_a_reused_pane(#[case] force: bool) {
        let runtime = FakeRuntime {
            pane_session_id: Some("different-session".to_string()),
            ..FakeRuntime::default()
        };

        let result = close_session(&session(), force, &runtime);

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            (
                "Pane `%42` is no longer bound to agent session `session-1`".to_string(),
                vec![pane_exists_call(), pane_session_call(),],
            ),
        );
    }

    #[test]
    fn close_does_not_kill_the_pane_when_sigterm_does_not_exit() {
        let runtime = FakeRuntime {
            graceful_result: Ok(false),
            wait_for_exit: false,
            ..FakeRuntime::default()
        };

        let result = close_session(&session(), false, &runtime);

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            (
                "Agent session `session-1` did not exit after SIGTERM; its pane was left open"
                    .to_string(),
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    resolve_pid_call(Engine::Claude),
                    draft_call(Engine::Claude),
                    graceful_quit_call(42),
                    Call::Sigterm(42),
                    Call::WaitForExit(42),
                ],
            ),
        );
    }

    #[test]
    fn close_refuses_to_remove_a_stopped_session_without_a_resolved_agent() {
        let runtime = FakeRuntime {
            draft: None,
            pid: None,
            ..FakeRuntime::default()
        };

        let result = close_session(&session(), false, &runtime);

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            (
                "Could not find a running agent process for session `session-1`; its pane was left open"
                    .to_string(),
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    resolve_pid_call(Engine::Claude),
                ],
            ),
        );
    }

    #[rstest]
    #[case::claude(Engine::Claude)]
    #[case::codex(Engine::Codex)]
    fn close_force_closes_a_matching_pane_when_agent_process_is_missing(
        mut session: Session,
        #[case] engine: Engine,
    ) {
        session.engine = engine;
        session.status = SessionStatus::Running;
        let runtime = FakeRuntime {
            pid: None,
            ..FakeRuntime::default()
        };

        let result = close_session(&session, true, &runtime);

        let mut expected_calls = vec![
            pane_exists_call(),
            pane_session_call(),
            resolve_pid_call(engine),
            pane_exists_call(),
            pane_session_call(),
        ];
        if engine == Engine::Codex {
            expected_calls.push(Call::EndCodexSession("session-1".to_string()));
        }
        expected_calls.push(kill_pane_call());

        assert_eq!(
            (result.is_ok(), runtime.calls.into_inner()),
            (true, expected_calls),
        );
    }

    #[rstest]
    #[case::claude_paused(Engine::Claude, SessionStatus::Paused)]
    #[case::codex_paused(Engine::Codex, SessionStatus::Paused)]
    #[case::claude_ended(Engine::Claude, SessionStatus::Ended)]
    #[case::codex_ended(Engine::Codex, SessionStatus::Ended)]
    fn close_removes_a_paused_or_ended_session_without_checking_for_a_draft(
        mut session: Session,
        #[case] engine: Engine,
        #[case] status: SessionStatus,
    ) {
        session.engine = engine;
        session.status = status;
        let runtime = FakeRuntime {
            draft: None,
            pid: None,
            ..FakeRuntime::default()
        };

        let result = close_session(&session, false, &runtime);
        let mut expected_calls = vec![
            pane_exists_call(),
            pane_session_call(),
            resolve_pid_call(engine),
            pane_exists_call(),
            pane_session_call(),
        ];
        if engine == Engine::Codex && status != SessionStatus::Ended {
            expected_calls.push(Call::EndCodexSession("session-1".to_string()));
        }
        expected_calls.push(kill_pane_call());

        assert_eq!(
            (result.is_ok(), runtime.calls.into_inner()),
            (true, expected_calls),
        );
    }

    #[test]
    fn close_command_has_the_ag_c_alias() {
        use clap::Parser;

        let parsed = crate::cli::Cli::try_parse_from([
            "a",
            "ag",
            "c",
            "feature/worktree",
            "--child",
            "--force",
            "--keep-worktree",
        ])
        .expect("the close command should parse");

        let close_args = match parsed.command {
            crate::cli::Commands::Agent(super::super::AgentCommands::Close(args)) => Some((
                args.target,
                args.child,
                args.force,
                args.skip_hooks,
                args.keep_worktree,
            )),
            _ => None,
        };

        assert_eq!(
            close_args,
            Some((
                Some("feature/worktree".to_string()),
                true,
                true,
                false,
                true
            ))
        );
    }

    #[test]
    fn keep_worktree_conflicts_with_skip_hooks() {
        use clap::Parser;

        let parsed = crate::cli::Cli::try_parse_from([
            "a",
            "ag",
            "c",
            "session-1",
            "--keep-worktree",
            "--skip-hooks",
        ]);

        assert_eq!(
            parsed.err().map(|error| error.kind()),
            Some(clap::error::ErrorKind::ArgumentConflict)
        );
    }

    #[test]
    fn child_flag_requires_a_target() {
        use clap::Parser;

        let parsed = crate::cli::Cli::try_parse_from(["a", "ag", "c", "--child"]);

        assert_eq!(
            parsed.err().map(|error| error.kind()),
            Some(clap::error::ErrorKind::MissingRequiredArgument)
        );
    }

    #[rstest]
    #[case::delete(vec!["a", "wm", "delete"])]
    #[case::d_alias(vec!["a", "wm", "d"])]
    #[case::rm_alias(vec!["a", "wm", "rm"])]
    #[case::clean(vec!["a", "wm", "clean"])]
    #[case::list(vec!["a", "wm", "list"])]
    fn wm_commands_are_removed(#[case] argv: Vec<&str>) {
        use clap::Parser;

        let parsed = crate::cli::Cli::try_parse_from(argv);

        assert_eq!(
            parsed.err().map(|error| error.kind()),
            Some(clap::error::ErrorKind::InvalidSubcommand)
        );
    }
}
