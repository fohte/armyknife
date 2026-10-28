//! `a agent close` shuts down a tracked agent session and removes its tmux pane.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use clap::Args;

mod child;
mod session_close;
mod worktree;
pub(crate) use worktree::close_worktree_path;
use worktree::{close_worktree, linked_worktree_root};
#[cfg(test)]
mod test_support;

use super::auto_pause;
use super::graceful_quit::GracefulQuitRequester;
use super::resume;
use super::signal::{LibcSignalSender, SignalSender};
use super::types::{Engine, Session, SessionStatus, TmuxInfo};
use crate::infra::process::{self, ProcessSnapshot};
use crate::infra::tmux;
use crate::shared::config::{load_config, load_config_or_default};
use crate::shared::env_var::EnvVars;
use crate::shared::worktree_delete;

const MAX_DESCENDANT_NODES: usize = 64;
const SIGTERM_GRACE_PERIOD: Duration = Duration::from_secs(5);
const DEFAULT_AUTO_PAUSE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Args, Clone, Default, PartialEq, Eq)]
pub struct CloseArgs {
    /// Session ID, worktree name, or worktree path to close.
    /// Defaults to the session in the current pane or the current worktree.
    pub target: Option<String>,

    /// Refuse to close the target unless it is a direct child session.
    #[arg(long, requires = "target")]
    pub child: bool,

    /// Override session safety checks and unmerged-worktree confirmation.
    #[arg(long)]
    pub force: bool,

    /// Skip worktree deletion hooks.
    #[arg(long)]
    pub skip_hooks: bool,
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
    match resolve_default_target(
        resume::resolve_session_id_from_pane(),
        super::store::load_session,
        current_worktree_root,
    )? {
        DefaultTarget::Session(session) => {
            session_close::close_session_value(&session, args, None).await
        }
        DefaultTarget::Worktree(worktree_root) => {
            close_worktree(&worktree_root, args, false, None).await
        }
    }
}

#[derive(Debug)]
enum DefaultTarget {
    Session(Box<Session>),
    Worktree(PathBuf),
}

fn resolve_default_target(
    pane_session_id: Result<String>,
    load_session: impl FnOnce(&str) -> Result<Option<Session>>,
    current_worktree_root: impl FnOnce() -> Result<Option<PathBuf>>,
) -> Result<DefaultTarget> {
    match pane_session_id {
        Ok(session_id) => match load_session(&session_id)? {
            Some(session) => Ok(DefaultTarget::Session(Box::new(session))),
            None => current_worktree_root()?
                .map(DefaultTarget::Worktree)
                .ok_or_else(|| anyhow::anyhow!("Agent session `{session_id}` was not found")),
        },
        Err(error) => current_worktree_root()?
            .map(DefaultTarget::Worktree)
            .ok_or(error),
    }
}

async fn close_target(
    target: &str,
    args: &CloseArgs,
    child_parent_session_id: Option<&str>,
) -> Result<()> {
    if !target.contains('/')
        && !target.contains('\\')
        && !target.contains("..")
        && let Some(session) = super::store::load_session(target)?
    {
        return session_close::close_session_value(&session, args, child_parent_session_id).await;
    }

    let config = load_config()?;
    let worktree_path = worktree_delete::resolve_worktree_path(
        target,
        &config.agent.worktree.dir,
        &config.agent.worktree.branch_prefix,
    )?;
    let worktree_path = PathBuf::from(worktree_path);
    let worktree_root = linked_worktree_root(&worktree_path).unwrap_or(worktree_path);
    close_worktree(&worktree_root, args, false, child_parent_session_id).await
}

fn close_tracked_session(session: &Session, force: bool) -> Result<()> {
    let mut session = session.clone();
    super::session_status::include_pending_status(&mut session);
    close_session(&session, force, &LiveCloseRuntime)
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
    tmux_info: &'a TmuxInfo,
    agent_pid: Option<u32>,
    should_end_codex_session: bool,
    draft_state_unavailable: bool,
}

fn prepare_close_session<'a, R: CloseRuntime>(
    session: &'a Session,
    force: bool,
    runtime: &R,
) -> Result<Option<SessionClosePlan<'a>>> {
    let tmux_info = session
        .tmux_info
        .as_ref()
        .context("Agent session has no tmux pane")?;
    if !runtime.pane_exists(tmux_info)? {
        return Ok(None);
    }
    ensure_pane_session(runtime, tmux_info, &session.session_id)?;

    if !force {
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

    let (agent_pid, should_end_codex_session, draft_state_unavailable) = match runtime
        .resolve_agent_pid(tmux_info, session.engine)?
    {
        Some(pid) => {
            let draft_state_unavailable = if force {
                false
            } else {
                ensure_no_unsent_draft(runtime, tmux_info, session)?
            };
            (Some(pid), true, draft_state_unavailable)
        }
        None if force => (None, true, false),
        None if !matches!(session.status, SessionStatus::Ended | SessionStatus::Paused) => {
            bail!(
                "Could not find a running agent process for session `{}`; its pane was left open",
                session.session_id
            );
        }
        None => (None, true, false),
    };

    Ok(Some(SessionClosePlan {
        tmux_info,
        agent_pid,
        should_end_codex_session,
        draft_state_unavailable,
    }))
}

fn close_session<R: CloseRuntime>(session: &Session, force: bool, runtime: &R) -> Result<()> {
    let stderr = io::stderr();
    close_session_with_warning_writer(session, force, runtime, &mut stderr.lock())
}

fn close_session_with_warning_writer<R: CloseRuntime>(
    session: &Session,
    force: bool,
    runtime: &R,
    warning_writer: &mut impl io::Write,
) -> Result<()> {
    let Some(plan) = prepare_close_session(session, force, runtime)? else {
        return Ok(());
    };
    let tmux_info = plan.tmux_info;
    let pane_id = tmux_info.pane_id.as_str();

    if plan.draft_state_unavailable
        && let Err(error) = write_draft_unavailable_warning(warning_writer, &session.session_id)
    {
        tracing::warn!(
            event = "agent.close.draft_state_warning_failed",
            session = %session.session_id,
            error = %error,
        );
    }

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

fn ensure_session_is_idle(session: &Session, effective_status: SessionStatus) -> Result<()> {
    if matches!(
        effective_status,
        SessionStatus::Running | SessionStatus::WaitingInput
    ) {
        bail!(
            "Agent session `{}` is running or waiting for input; pass --force to close it",
            session.session_id
        );
    }
    if session.has_pending_bg_tasks() {
        bail!(
            "Agent session `{}` has pending background or agent tasks; pass --force to close it",
            session.session_id
        );
    }
    Ok(())
}

fn ensure_no_unsent_draft<R: CloseRuntime>(
    runtime: &R,
    tmux_info: &TmuxInfo,
    session: &Session,
) -> Result<bool> {
    match runtime.has_draft(tmux_info, session.engine) {
        Some(false) => Ok(false),
        Some(true) => bail!(
            "Agent session `{}` has an unsent draft; pass --force to close it",
            session.session_id
        ),
        None => {
            let timeout = runtime.unknown_draft_timeout();
            let active = match Utc::now()
                .signed_duration_since(session.updated_at)
                .to_std()
            {
                Ok(elapsed) => elapsed < timeout,
                Err(_) => true,
            };
            tracing::debug!(
                event = "agent.close.draft_state_unavailable",
                session = %session.session_id,
                timeout = ?timeout,
                decision = if active { "blocked_recent_activity" } else { "proceed_after_timeout" },
            );
            if active {
                bail!(
                    "Could not check for an unsent draft in agent session `{}`; recent activity prevents closing it, pass --force to close it",
                    session.session_id
                );
            }
            Ok(true)
        }
    }
}

fn write_draft_unavailable_warning(
    writer: &mut impl io::Write,
    session_id: &str,
) -> io::Result<()> {
    writeln!(
        writer,
        "Warning: could not inspect the unsent draft for session `{session_id}`; using its last update time for the close safety check"
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io;
    use std::path::PathBuf;

    use rstest::rstest;

    use super::test_support::session;
    use super::*;
    use crate::commands::agent::types::TmuxInfo;

    #[derive(Debug, PartialEq, Eq)]
    enum Call {
        PaneExists(Option<String>, String),
        PaneSession(Option<String>, String),
        Draft(Option<String>, String, Engine),
        ResolvePid(Option<String>, String, Engine),
        LatestTurnStatus(String),
        GracefulQuit(Option<String>, String, u32),
        Sigterm(u32),
        WaitForExit(u32),
        EndCodexSession(String),
        KillPane(Option<String>, String),
    }

    struct FakeRuntime {
        calls: RefCell<Vec<Call>>,
        pane_exists: Result<bool>,
        pane_session_id: Option<String>,
        draft: Option<bool>,
        turn_status: std::result::Result<Option<String>, String>,
        pid: Option<u32>,
        pid_error: Option<String>,
        graceful_result: io::Result<bool>,
        signal_result: io::Result<()>,
        wait_for_exit: bool,
    }

    impl Default for FakeRuntime {
        fn default() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                pane_exists: Ok(true),
                pane_session_id: Some("session-1".to_string()),
                draft: Some(false),
                turn_status: Ok(None),
                pid: Some(42),
                pid_error: None,
                graceful_result: Ok(true),
                signal_result: Ok(()),
                wait_for_exit: true,
            }
        }
    }

    impl CloseRuntime for FakeRuntime {
        fn pane_exists(&self, tmux_info: &TmuxInfo) -> Result<bool> {
            self.calls.borrow_mut().push(Call::PaneExists(
                tmux_info.socket_path.clone(),
                tmux_info.pane_id.clone(),
            ));
            match &self.pane_exists {
                Ok(exists) => Ok(*exists),
                Err(error) => Err(anyhow::anyhow!(error.to_string())),
            }
        }

        fn pane_session_id(&self, tmux_info: &TmuxInfo) -> Option<String> {
            self.calls.borrow_mut().push(Call::PaneSession(
                tmux_info.socket_path.clone(),
                tmux_info.pane_id.clone(),
            ));
            self.pane_session_id.clone()
        }

        fn has_draft(&self, tmux_info: &TmuxInfo, engine: Engine) -> Option<bool> {
            self.calls.borrow_mut().push(Call::Draft(
                tmux_info.socket_path.clone(),
                tmux_info.pane_id.clone(),
                engine,
            ));
            self.draft
        }

        fn latest_codex_turn_status(&self, thread_id: &str) -> Result<Option<String>> {
            self.calls
                .borrow_mut()
                .push(Call::LatestTurnStatus(thread_id.to_string()));
            match &self.turn_status {
                Ok(status) => Ok(status.clone()),
                Err(error) => bail!(error.clone()),
            }
        }

        fn resolve_agent_pid(&self, tmux_info: &TmuxInfo, engine: Engine) -> Result<Option<u32>> {
            self.calls.borrow_mut().push(Call::ResolvePid(
                tmux_info.socket_path.clone(),
                tmux_info.pane_id.clone(),
                engine,
            ));
            if let Some(error) = &self.pid_error {
                return Err(anyhow::anyhow!(error.clone()));
            }
            Ok(self.pid)
        }

        fn request_graceful_quit(&self, tmux_info: &TmuxInfo, pid: u32) -> io::Result<bool> {
            self.calls.borrow_mut().push(Call::GracefulQuit(
                tmux_info.socket_path.clone(),
                tmux_info.pane_id.clone(),
                pid,
            ));
            match &self.graceful_result {
                Ok(result) => Ok(*result),
                Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
            }
        }

        fn send_sigterm(&self, pid: u32) -> io::Result<()> {
            self.calls.borrow_mut().push(Call::Sigterm(pid));
            match &self.signal_result {
                Ok(()) => Ok(()),
                Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
            }
        }

        fn wait_for_exit(&self, pid: u32) -> bool {
            self.calls.borrow_mut().push(Call::WaitForExit(pid));
            self.wait_for_exit
        }

        fn end_codex_session(&self, session_id: &str) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(Call::EndCodexSession(session_id.to_string()));
            Ok(())
        }

        fn kill_pane(&self, tmux_info: &TmuxInfo) -> Result<()> {
            self.calls.borrow_mut().push(Call::KillPane(
                tmux_info.socket_path.clone(),
                tmux_info.pane_id.clone(),
            ));
            Ok(())
        }
    }

    fn pane_exists_call() -> Call {
        Call::PaneExists(test_socket_path(), "%42".to_string())
    }

    fn resolve_pid_call(engine: Engine) -> Call {
        Call::ResolvePid(test_socket_path(), "%42".to_string(), engine)
    }

    fn test_socket_path() -> Option<String> {
        Some("/tmp/tmux-test/socket".to_string())
    }

    fn pane_session_call() -> Call {
        Call::PaneSession(test_socket_path(), "%42".to_string())
    }

    fn draft_call(engine: Engine) -> Call {
        Call::Draft(test_socket_path(), "%42".to_string(), engine)
    }

    fn graceful_quit_call(pid: u32) -> Call {
        Call::GracefulQuit(test_socket_path(), "%42".to_string(), pid)
    }

    fn kill_pane_call() -> Call {
        Call::KillPane(test_socket_path(), "%42".to_string())
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
                    "Agent session `session-1` is running or waiting for input; pass --force to close it"
                } else {
                    "Agent session `session-1` has pending background or agent tasks; pass --force to close it"
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
                "Agent session `session-1` is running or waiting for input; pass --force to close it"
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
                "Agent session `session-1` has pending background or agent tasks; pass --force to close it"
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
                "Agent session `session-1` is running or waiting for input; pass --force to close it"
                    .to_string(),
                vec![
                    pane_exists_call(),
                    pane_session_call(),
                    Call::LatestTurnStatus("session-1".to_string()),
                ],
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
    #[case::stopped(SessionStatus::Stopped)]
    #[case::paused(SessionStatus::Paused)]
    #[case::ended(SessionStatus::Ended)]
    fn close_allows_unknown_draft_when_session_update_is_stale(
        mut session: Session,
        #[case] status: SessionStatus,
    ) {
        session.status = status;
        session.updated_at = Utc::now() - chrono::TimeDelta::hours(1);
        let runtime = FakeRuntime {
            draft: None,
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

    #[rstest]
    #[case::stored_pane_session_wins(
        Some("session-1"),
        true,
        Some("/tmp/worktrees/feature"),
        Ok(ResolvedTarget::SessionId("session-1".to_string()))
    )]
    #[case::stale_pane_session_falls_back_to_worktree(
        Some("session-1"),
        false,
        Some("/tmp/worktrees/feature"),
        Ok(ResolvedTarget::Worktree(PathBuf::from("/tmp/worktrees/feature")))
    )]
    #[case::stale_pane_session_preserves_not_found_without_worktree(
        Some("session-1"),
        false,
        None,
        Err("Agent session `session-1` was not found".to_string())
    )]
    #[case::unavailable_pane_session_falls_back_to_worktree(
        None,
        false,
        Some("/tmp/worktrees/feature"),
        Ok(ResolvedTarget::Worktree(PathBuf::from("/tmp/worktrees/feature")))
    )]
    #[case::preserves_pane_error_without_worktree(
        None,
        false,
        None,
        Err("no agent session for pane".to_string())
    )]
    fn resolves_default_target_from_pane_or_current_worktree(
        #[case] pane_session_id: Option<&str>,
        #[case] session_is_stored: bool,
        #[case] worktree_root: Option<&str>,
        #[case] expected: std::result::Result<ResolvedTarget, String>,
    ) {
        let pane_session_id = pane_session_id
            .map(|session_id| Ok(session_id.to_string()))
            .unwrap_or_else(|| Err(anyhow::anyhow!("no agent session for pane")));
        let worktree_root = worktree_root.map(PathBuf::from);
        let actual = resolve_default_target(
            pane_session_id,
            |_| Ok(session_is_stored.then(session)),
            || Ok(worktree_root),
        )
        .map(|target| match target {
            DefaultTarget::Session(session) => ResolvedTarget::SessionId(session.session_id),
            DefaultTarget::Worktree(path) => ResolvedTarget::Worktree(path),
        })
        .map_err(|error| error.to_string());

        assert_eq!(actual, expected);
    }

    #[derive(Debug, PartialEq, Eq)]
    enum ResolvedTarget {
        SessionId(String),
        Worktree(PathBuf),
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
            "--skip-hooks",
        ])
        .expect("the close command should parse");

        let close_args = match parsed.command {
            crate::cli::Commands::Agent(super::super::AgentCommands::Close(args)) => {
                Some((args.target, args.child, args.force, args.skip_hooks))
            }
            _ => None,
        };

        assert_eq!(
            close_args,
            Some((Some("feature/worktree".to_string()), true, true, true))
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
