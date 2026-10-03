//! `a agent close` shuts down a tracked agent session and removes its tmux pane.

use std::io;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;

use super::graceful_quit::GracefulQuitRequester;
use super::resume;
use super::signal::{LibcSignalSender, SignalSender};
use super::types::{Engine, Session, SessionStatus};
use crate::infra::git::GitRepo;
use crate::infra::process::{self, ProcessSnapshot};
use crate::infra::tmux;

const MAX_DESCENDANT_NODES: usize = 64;
const SIGTERM_GRACE_PERIOD: Duration = Duration::from_secs(5);

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CloseArgs {
    /// Session ID to close (defaults to the session in the current pane).
    pub session_id: Option<String>,

    /// Close the session even when it is active, has pending tasks, or has an unsent draft.
    #[arg(long)]
    pub force: bool,
}

pub fn run(args: &CloseArgs) -> Result<()> {
    let session_id = match args.session_id.as_deref() {
        Some(session_id) => session_id.to_string(),
        None => resume::resolve_session_id_from_pane()?,
    };
    let Some(mut session) = super::store::load_session(&session_id)? else {
        bail!("Agent session `{session_id}` was not found");
    };

    reject_linked_worktree(&session)?;
    super::session_status::include_pending_status(&mut session);
    close_session(&session, args.force, &LiveCloseRuntime)
}

fn reject_linked_worktree(session: &Session) -> Result<()> {
    let is_worktree = GitRepo::open_at(&session.cwd).is_ok_and(|repo| repo.is_worktree());
    ensure_not_linked_worktree(session, is_worktree)
}

fn ensure_not_linked_worktree(session: &Session, is_worktree: bool) -> Result<()> {
    if is_worktree {
        bail!(
            "Cannot close agent session `{}` because its working directory is a linked worktree",
            session.session_id
        );
    }
    Ok(())
}

trait CloseRuntime {
    fn pane_exists(&self, pane_id: &str) -> Result<bool>;
    fn pane_session_id(&self, pane_id: &str) -> Option<String>;
    fn has_draft(&self, pane_id: &str, engine: Engine) -> Option<bool>;
    fn resolve_agent_pid(&self, pane_id: &str, engine: Engine) -> Result<Option<u32>>;
    fn request_graceful_quit(&self, pane_id: &str, pid: u32) -> io::Result<bool>;
    fn send_sigterm(&self, pid: u32) -> io::Result<()>;
    fn wait_for_exit(&self, pid: u32) -> bool;
    fn kill_pane(&self, pane_id: &str) -> Result<()>;
}

struct LiveCloseRuntime;

impl CloseRuntime for LiveCloseRuntime {
    fn pane_exists(&self, pane_id: &str) -> Result<bool> {
        Ok(tmux::list_all_pane_ids()?.contains(pane_id))
    }

    fn pane_session_id(&self, pane_id: &str) -> Option<String> {
        super::types::resolve_session_option(|option| tmux::get_pane_option(pane_id, option))
    }

    fn has_draft(&self, pane_id: &str, engine: Engine) -> Option<bool> {
        super::pane::input::pane_has_draft(pane_id, engine)
    }

    fn resolve_agent_pid(&self, pane_id: &str, engine: Engine) -> Result<Option<u32>> {
        let pane_pid = tmux::get_pane_pid(pane_id)
            .with_context(|| format!("Could not resolve the process for pane `{pane_id}`"))?;
        let snapshot = ProcessSnapshot::capture().context("Could not inspect running processes")?;
        Ok(snapshot.find_self_or_descendant_by_command(
            pane_pid,
            engine.process_name(),
            MAX_DESCENDANT_NODES,
        ))
    }

    fn request_graceful_quit(&self, pane_id: &str, pid: u32) -> io::Result<bool> {
        LibcSignalSender.request_and_wait(pane_id, pid)
    }

    fn send_sigterm(&self, pid: u32) -> io::Result<()> {
        LibcSignalSender.send(pid, libc::SIGTERM)
    }

    fn wait_for_exit(&self, pid: u32) -> bool {
        process::wait_for_process_exit(pid, SIGTERM_GRACE_PERIOD)
    }

    fn kill_pane(&self, pane_id: &str) -> Result<()> {
        Ok(tmux::kill_pane(pane_id)?)
    }
}

fn close_session<R: CloseRuntime>(session: &Session, force: bool, runtime: &R) -> Result<()> {
    let pane_id = session
        .tmux_info
        .as_ref()
        .map(|info| info.pane_id.as_str())
        .context("Agent session has no tmux pane")?;

    if !runtime.pane_exists(pane_id)? {
        return Ok(());
    }
    ensure_pane_session(runtime, pane_id, &session.session_id)?;

    if !force {
        ensure_session_is_idle(session)?;
        match runtime.has_draft(pane_id, session.engine) {
            Some(false) => {}
            Some(true) => bail!(
                "Agent session `{}` has an unsent draft; pass --force to close it",
                session.session_id
            ),
            None => bail!(
                "Could not check for an unsent draft in agent session `{}`; pass --force to close it",
                session.session_id
            ),
        }
    }

    match runtime.resolve_agent_pid(pane_id, session.engine)? {
        Some(pid) => {
            let exited = match runtime.request_graceful_quit(pane_id, pid) {
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
        None if !matches!(session.status, SessionStatus::Ended | SessionStatus::Paused) => {
            bail!(
                "Could not find a running agent process for session `{}`; its pane was left open",
                session.session_id
            );
        }
        None => {}
    }

    if runtime.pane_exists(pane_id)? {
        ensure_pane_session(runtime, pane_id, &session.session_id)?;
        runtime.kill_pane(pane_id)?;
    }
    Ok(())
}

fn ensure_pane_session<R: CloseRuntime>(
    runtime: &R,
    pane_id: &str,
    session_id: &str,
) -> Result<()> {
    if runtime.pane_session_id(pane_id).as_deref() != Some(session_id) {
        bail!("Pane `{pane_id}` is no longer bound to agent session `{session_id}`");
    }
    Ok(())
}

fn ensure_session_is_idle(session: &Session) -> Result<()> {
    if matches!(
        session.status,
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeSet;
    use std::io;
    use std::path::PathBuf;

    use chrono::Utc;
    use rstest::{fixture, rstest};

    use super::*;
    use crate::commands::agent::types::TmuxInfo;

    #[derive(Debug, PartialEq, Eq)]
    enum Call {
        PaneExists(String),
        PaneSession(String),
        Draft(String, Engine),
        ResolvePid(String, Engine),
        GracefulQuit(String, u32),
        Sigterm(u32),
        WaitForExit(u32),
        KillPane(String),
    }

    struct FakeRuntime {
        calls: RefCell<Vec<Call>>,
        pane_exists: Result<bool>,
        pane_session_id: Option<String>,
        draft: Option<bool>,
        pid: Option<u32>,
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
                pid: Some(42),
                graceful_result: Ok(true),
                signal_result: Ok(()),
                wait_for_exit: true,
            }
        }
    }

    impl CloseRuntime for FakeRuntime {
        fn pane_exists(&self, pane_id: &str) -> Result<bool> {
            self.calls
                .borrow_mut()
                .push(Call::PaneExists(pane_id.to_string()));
            match &self.pane_exists {
                Ok(exists) => Ok(*exists),
                Err(error) => Err(anyhow::anyhow!(error.to_string())),
            }
        }

        fn pane_session_id(&self, pane_id: &str) -> Option<String> {
            self.calls
                .borrow_mut()
                .push(Call::PaneSession(pane_id.to_string()));
            self.pane_session_id.clone()
        }

        fn has_draft(&self, pane_id: &str, engine: Engine) -> Option<bool> {
            self.calls
                .borrow_mut()
                .push(Call::Draft(pane_id.to_string(), engine));
            self.draft
        }

        fn resolve_agent_pid(&self, pane_id: &str, engine: Engine) -> Result<Option<u32>> {
            self.calls
                .borrow_mut()
                .push(Call::ResolvePid(pane_id.to_string(), engine));
            Ok(self.pid)
        }

        fn request_graceful_quit(&self, pane_id: &str, pid: u32) -> io::Result<bool> {
            self.calls
                .borrow_mut()
                .push(Call::GracefulQuit(pane_id.to_string(), pid));
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

        fn kill_pane(&self, pane_id: &str) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(Call::KillPane(pane_id.to_string()));
            Ok(())
        }
    }

    #[fixture]
    fn session() -> Session {
        Session {
            session_id: "session-1".to_string(),
            crit_urls: Vec::new(),
            cwd: PathBuf::from("/tmp/test-repo"),
            transcript_path: None,
            tty: None,
            tmux_info: Some(TmuxInfo {
                session_name: "test".to_string(),
                window_name: "agent".to_string(),
                window_index: 1,
                pane_id: "%42".to_string(),
            }),
            status: SessionStatus::Stopped,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            engine: Engine::Claude,
        }
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
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                ],
            ),
        );
    }

    #[rstest]
    #[case::draft_present(
        Some(true),
        "Agent session `session-1` has an unsent draft; pass --force to close it"
    )]
    #[case::draft_unknown(
        None,
        "Could not check for an unsent draft in agent session `session-1`; pass --force to close it"
    )]
    fn close_rejects_unsent_or_uninspectable_drafts(
        session: Session,
        #[case] draft: Option<bool>,
        #[case] expected_error: &str,
    ) {
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
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                    Call::Draft("%42".to_string(), Engine::Claude),
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
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                    Call::Draft("%42".to_string(), Engine::Claude),
                    Call::ResolvePid("%42".to_string(), Engine::Claude),
                    Call::GracefulQuit("%42".to_string(), 42),
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                    Call::KillPane("%42".to_string()),
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
            (
                "tmux is unavailable".to_string(),
                vec![Call::PaneExists("%42".to_string())],
            ),
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
            (true, vec![Call::PaneExists("%42".to_string())]),
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
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                    Call::Draft("%42".to_string(), Engine::Claude),
                    Call::ResolvePid("%42".to_string(), Engine::Claude),
                    Call::GracefulQuit("%42".to_string(), 42),
                    Call::Sigterm(42),
                    Call::WaitForExit(42),
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                    Call::KillPane("%42".to_string()),
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
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                    Call::Draft("%42".to_string(), Engine::Claude),
                    Call::ResolvePid("%42".to_string(), Engine::Claude),
                    Call::GracefulQuit("%42".to_string(), 42),
                    Call::Sigterm(42),
                    Call::WaitForExit(42),
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                    Call::KillPane("%42".to_string()),
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
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                    Call::ResolvePid("%42".to_string(), Engine::Claude),
                    Call::GracefulQuit("%42".to_string(), 42),
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                    Call::KillPane("%42".to_string()),
                ],
            ),
        );
    }

    #[test]
    fn close_refuses_to_kill_a_reused_pane() {
        let runtime = FakeRuntime {
            pane_session_id: Some("different-session".to_string()),
            ..FakeRuntime::default()
        };

        let result = close_session(&session(), false, &runtime);

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            (
                "Pane `%42` is no longer bound to agent session `session-1`".to_string(),
                vec![
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                ],
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
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                    Call::Draft("%42".to_string(), Engine::Claude),
                    Call::ResolvePid("%42".to_string(), Engine::Claude),
                    Call::GracefulQuit("%42".to_string(), 42),
                    Call::Sigterm(42),
                    Call::WaitForExit(42),
                ],
            ),
        );
    }

    #[rstest]
    #[case::normal(false)]
    #[case::forced(true)]
    fn close_refuses_to_remove_a_stopped_session_without_a_resolved_agent(
        session: Session,
        #[case] force: bool,
    ) {
        let runtime = FakeRuntime {
            pid: None,
            ..FakeRuntime::default()
        };

        let result = close_session(&session, force, &runtime);

        let expected_calls = if force {
            vec![
                Call::PaneExists("%42".to_string()),
                Call::PaneSession("%42".to_string()),
                Call::ResolvePid("%42".to_string(), Engine::Claude),
            ]
        } else {
            vec![
                Call::PaneExists("%42".to_string()),
                Call::PaneSession("%42".to_string()),
                Call::Draft("%42".to_string(), Engine::Claude),
                Call::ResolvePid("%42".to_string(), Engine::Claude),
            ]
        };

        assert_eq!(
            (result.unwrap_err().to_string(), runtime.calls.into_inner()),
            (
                "Could not find a running agent process for session `session-1`; its pane was left open"
                    .to_string(),
                expected_calls,
            ),
        );
    }

    #[test]
    fn close_removes_the_pane_when_the_session_already_ended() {
        let mut session = session();
        session.status = SessionStatus::Ended;
        let runtime = FakeRuntime {
            pid: None,
            ..FakeRuntime::default()
        };

        let result = close_session(&session, false, &runtime);

        assert_eq!(
            (result.is_ok(), runtime.calls.into_inner()),
            (
                true,
                vec![
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                    Call::Draft("%42".to_string(), Engine::Claude),
                    Call::ResolvePid("%42".to_string(), Engine::Claude),
                    Call::PaneExists("%42".to_string()),
                    Call::PaneSession("%42".to_string()),
                    Call::KillPane("%42".to_string()),
                ],
            ),
        );
    }

    #[test]
    fn linked_worktree_sessions_are_rejected() {
        let result = ensure_not_linked_worktree(&session(), true);

        assert_eq!(
            result.unwrap_err().to_string(),
            "Cannot close agent session `session-1` because its working directory is a linked worktree"
        );
    }

    #[test]
    fn close_command_has_the_ag_c_alias() {
        use clap::Parser;

        let parsed = crate::cli::Cli::try_parse_from(["a", "ag", "c", "session-1", "--force"])
            .expect("the close command should parse");

        let close_args = match parsed.command {
            crate::cli::Commands::Agent(super::super::AgentCommands::Close(args)) => {
                Some((args.session_id, args.force))
            }
            _ => None,
        };

        assert_eq!(close_args, Some((Some("session-1".to_string()), true)));
    }
}
