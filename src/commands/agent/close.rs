//! `a agent close` shuts down a tracked agent session and removes its tmux pane.

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
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
use crate::shared::config::load_config;
use crate::shared::worktree_delete;

const MAX_DESCENDANT_NODES: usize = 64;
const SIGTERM_GRACE_PERIOD: Duration = Duration::from_secs(5);

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CloseArgs {
    /// Session ID, worktree name, or worktree path to close.
    /// Defaults to the session in the current pane or the current worktree.
    pub target: Option<String>,

    /// Override session safety checks and unmerged-worktree confirmation.
    #[arg(long)]
    pub force: bool,

    /// Skip the pre-worktree-delete hook.
    #[arg(long)]
    pub skip_hooks: bool,
}

pub async fn run(args: &CloseArgs) -> Result<()> {
    if let Some(target) = args.target.as_deref() {
        return close_target(target, args).await;
    }

    let pane_session_id = resume::resolve_session_id_from_pane();
    let worktree_root = if pane_session_id.is_err() {
        let cwd = std::env::current_dir().context("Failed to get current directory")?;
        linked_worktree_root(&cwd)
    } else {
        None
    };
    match resolve_default_target(pane_session_id, worktree_root)? {
        DefaultTarget::SessionId(session_id) => close_session_target(&session_id, args).await,
        DefaultTarget::Worktree(worktree_root) => close_worktree(&worktree_root, args).await,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum DefaultTarget {
    SessionId(String),
    Worktree(PathBuf),
}

fn resolve_default_target(
    pane_session_id: Result<String>,
    current_worktree_root: Option<PathBuf>,
) -> Result<DefaultTarget> {
    match pane_session_id {
        Ok(session_id) => Ok(DefaultTarget::SessionId(session_id)),
        Err(error) => current_worktree_root
            .map(DefaultTarget::Worktree)
            .ok_or(error),
    }
}

async fn close_target(target: &str, args: &CloseArgs) -> Result<()> {
    if !target.contains('/')
        && !target.contains('\\')
        && !target.contains("..")
        && let Some(session) = super::store::load_session(target)?
    {
        return close_session_value(&session, args).await;
    }

    let config = load_config()?;
    let worktree_path = worktree_delete::resolve_worktree_path(
        target,
        &config.agent.worktree.dir,
        &config.agent.worktree.branch_prefix,
    )?;
    let worktree_path = PathBuf::from(worktree_path);
    let worktree_root = linked_worktree_root(&worktree_path).unwrap_or(worktree_path);
    close_worktree(&worktree_root, args).await
}

async fn close_session_target(session_id: &str, args: &CloseArgs) -> Result<()> {
    let Some(session) = super::store::load_session(session_id)? else {
        bail!("Agent session `{session_id}` was not found");
    };
    close_session_value(&session, args).await
}

async fn close_session_value(session: &Session, args: &CloseArgs) -> Result<()> {
    let Some(worktree_root) = linked_worktree_root(&session.cwd) else {
        return close_tracked_session(session, args.force);
    };

    close_with_worktree_plan(
        worktree_delete::prepare(&worktree_root, args.force),
        || close_worktree_session(session, args.force),
        |plan| worktree_delete::execute(plan, args.skip_hooks),
    )
    .await
}

async fn close_worktree(worktree_root: &Path, args: &CloseArgs) -> Result<()> {
    let sessions = super::store::list_sessions()?;
    let session = select_worktree_session(&sessions, worktree_root)?;
    close_with_worktree_plan(
        worktree_delete::prepare(worktree_root, args.force),
        || {
            if let Some(session) = session {
                close_worktree_session(session, args.force)?;
            }
            Ok(())
        },
        |plan| worktree_delete::execute(plan, args.skip_hooks),
    )
    .await
}

async fn close_with_worktree_plan<P, F>(
    prepare: impl Future<Output = Result<P>>,
    close_session: impl FnOnce() -> Result<()>,
    remove_worktree: impl FnOnce(P) -> F,
) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    let plan = prepare.await?;
    close_session()?;
    remove_worktree(plan).await
}

fn close_tracked_session(session: &Session, force: bool) -> Result<()> {
    let mut session = session.clone();
    super::session_status::include_pending_status(&mut session);
    close_session(&session, force, &LiveCloseRuntime)
}

fn close_worktree_session(session: &Session, force: bool) -> Result<()> {
    let current_pane_id = tmux::current_pane_id_from_env();
    if !should_gracefully_close_worktree_session(session, current_pane_id.as_deref()) {
        return Ok(());
    }
    close_tracked_session(session, force)
}

fn should_gracefully_close_worktree_session(
    session: &Session,
    current_pane_id: Option<&str>,
) -> bool {
    session
        .tmux_info
        .as_ref()
        .is_some_and(|info| current_pane_id != Some(info.pane_id.as_str()))
}

fn linked_worktree_root(path: &Path) -> Option<PathBuf> {
    let repo = GitRepo::open_at(path).ok()?;
    repo.is_worktree().then(|| repo.workdir().to_path_buf())
}

fn path_is_within_worktree(path: &Path, worktree_root: &Path) -> bool {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let worktree_root = worktree_root
        .canonicalize()
        .unwrap_or_else(|_| worktree_root.to_path_buf());
    path.starts_with(worktree_root)
}

fn select_worktree_session<'a>(
    sessions: &'a [Session],
    worktree_root: &Path,
) -> Result<Option<&'a Session>> {
    let mut matching = sessions
        .iter()
        .filter(|session| path_is_within_worktree(&session.cwd, worktree_root));
    let Some(session) = matching.next() else {
        return Ok(None);
    };
    if matching.next().is_some() {
        bail!(
            "Multiple agent sessions are associated with worktree `{}`; pass a session ID to select one",
            worktree_root.display()
        );
    }
    Ok(Some(session))
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
            work_type: None,
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

    #[rstest]
    #[case::root(vec!["/tmp/worktrees/feature"], Ok(Some("session-0".to_string())))]
    #[case::nested(vec!["/tmp/worktrees/feature/src"], Ok(Some("session-0".to_string())))]
    #[case::sibling(vec!["/tmp/worktrees/feature-extra"], Ok(None))]
    #[case::multiple(
        vec!["/tmp/worktrees/feature", "/tmp/worktrees/feature/src"],
        Err("Multiple agent sessions are associated with worktree `/tmp/worktrees/feature`; pass a session ID to select one".to_string())
    )]
    fn selects_a_unique_session_for_the_target_worktree(
        #[case] session_cwds: Vec<&str>,
        #[case] expected: std::result::Result<Option<String>, String>,
    ) {
        let sessions = session_cwds
            .into_iter()
            .enumerate()
            .map(|(index, cwd)| {
                let mut session = session();
                session.session_id = format!("session-{index}");
                session.cwd = PathBuf::from(cwd);
                session
            })
            .collect::<Vec<_>>();
        let actual = select_worktree_session(&sessions, Path::new("/tmp/worktrees/feature"))
            .map(|session| session.map(|session| session.session_id.clone()))
            .map_err(|error| error.to_string());

        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case::same_pane(Some("%42"), true, false)]
    #[case::other_pane(Some("%99"), true, true)]
    #[case::no_current_pane(None, true, true)]
    #[case::missing_tmux_info(Some("%42"), false, false)]
    fn skips_graceful_close_for_the_current_or_untracked_pane(
        mut session: Session,
        #[case] current_pane_id: Option<&str>,
        #[case] has_tmux_info: bool,
        #[case] expected: bool,
    ) {
        if !has_tmux_info {
            session.tmux_info = None;
        }
        assert_eq!(
            should_gracefully_close_worktree_session(&session, current_pane_id),
            expected
        );
    }

    #[tokio::test]
    async fn cancelled_worktree_confirmation_leaves_the_session_open() {
        let calls = RefCell::new(Vec::new());

        let result = close_with_worktree_plan(
            async {
                calls.borrow_mut().push("prepare");
                Err::<(), _>(anyhow::anyhow!("Cancelled."))
            },
            || {
                calls.borrow_mut().push("close session");
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree");
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (result.unwrap_err().to_string(), calls.into_inner()),
            ("Cancelled.".to_string(), vec!["prepare"]),
        );
    }

    #[tokio::test]
    async fn worktree_close_removes_after_closing_the_selected_session() {
        let calls = RefCell::new(Vec::new());

        let result = close_with_worktree_plan(
            async {
                calls.borrow_mut().push("prepare");
                Ok(())
            },
            || {
                calls.borrow_mut().push("close session");
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree");
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (result.is_ok(), calls.into_inner()),
            (true, vec!["prepare", "close session", "remove worktree"]),
        );
    }

    #[rstest]
    #[case::pane_session_wins(Some("session-1"), Some("/tmp/worktrees/feature"), Ok(DefaultTarget::SessionId("session-1".to_string())))]
    #[case::worktree_fallback(
        None,
        Some("/tmp/worktrees/feature"),
        Ok(DefaultTarget::Worktree(PathBuf::from("/tmp/worktrees/feature")))
    )]
    #[case::preserves_pane_error(None, None, Err("no agent session for pane".to_string()))]
    fn resolves_default_target_from_pane_or_current_worktree(
        #[case] pane_session_id: Option<&str>,
        #[case] worktree_root: Option<&str>,
        #[case] expected: std::result::Result<DefaultTarget, String>,
    ) {
        let pane_session_id = pane_session_id
            .map(|session_id| Ok(session_id.to_string()))
            .unwrap_or_else(|| Err(anyhow::anyhow!("no agent session for pane")));
        let worktree_root = worktree_root.map(PathBuf::from);
        let actual = resolve_default_target(pane_session_id, worktree_root)
            .map_err(|error| error.to_string());

        assert_eq!(actual, expected);
    }

    #[test]
    fn close_command_has_the_ag_c_alias() {
        use clap::Parser;

        let parsed = crate::cli::Cli::try_parse_from([
            "a",
            "ag",
            "c",
            "feature/worktree",
            "--force",
            "--skip-hooks",
        ])
        .expect("the close command should parse");

        let close_args = match parsed.command {
            crate::cli::Commands::Agent(super::super::AgentCommands::Close(args)) => {
                Some((args.target, args.force, args.skip_hooks))
            }
            _ => None,
        };

        assert_eq!(
            close_args,
            Some((Some("feature/worktree".to_string()), true, true))
        );
    }

    #[rstest]
    #[case::delete(vec!["a", "wm", "delete"])]
    #[case::d_alias(vec!["a", "wm", "d"])]
    #[case::rm_alias(vec!["a", "wm", "rm"])]
    fn wm_delete_command_and_aliases_are_removed(#[case] argv: Vec<&str>) {
        use clap::Parser;

        let parsed = crate::cli::Cli::try_parse_from(argv);

        assert_eq!(
            parsed.err().map(|error| error.kind()),
            Some(clap::error::ErrorKind::InvalidSubcommand)
        );
    }
}
