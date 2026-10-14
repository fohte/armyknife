//! `a agent close` shuts down a tracked agent session and removes its tmux pane.

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;

mod child;

use super::graceful_quit::GracefulQuitRequester;
use super::resume;
use super::signal::{LibcSignalSender, SignalSender};
use super::types::{Engine, Session, SessionStatus, TmuxInfo};
use crate::infra::git::GitRepo;
use crate::infra::process::{self, ProcessSnapshot};
use crate::infra::tmux;
use crate::shared::config::load_config;
use crate::shared::env_var::EnvVars;
use crate::shared::worktree_delete;

const MAX_DESCENDANT_NODES: usize = 64;
const SIGTERM_GRACE_PERIOD: Duration = Duration::from_secs(5);

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
        DefaultTarget::Session(session) => close_session_value(&session, args, None).await,
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
        return close_session_value(&session, args, child_parent_session_id).await;
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

async fn close_session_value(
    session: &Session,
    args: &CloseArgs,
    child_parent_session_id: Option<&str>,
) -> Result<()> {
    let Some(worktree_root) = linked_worktree_root(&session.cwd) else {
        return close_session_value_without_worktree(
            session,
            args,
            child_parent_session_id,
            close_tracked_session,
        );
    };

    close_session_value_with_worktree(
        SessionWorktreeContext {
            session,
            force: args.force,
            parent_session_id: child_parent_session_id,
            worktree_root: &worktree_root,
        },
        worktree_delete::prepare(&worktree_root, args.force),
        super::store::load_session,
        close_tracked_session,
        || {
            let current_pane_id = crate::shared::caller_pane::resolve_caller_pane_id();
            close_worktree_session(session, args.force, current_pane_id.as_deref())
        },
        |plan| worktree_delete::execute(plan, args.skip_hooks),
    )
    .await
}

struct SessionWorktreeContext<'a> {
    session: &'a Session,
    force: bool,
    parent_session_id: Option<&'a str>,
    worktree_root: &'a Path,
}

async fn close_session_value_with_worktree<P, F>(
    context: SessionWorktreeContext<'_>,
    prepare_worktree: impl Future<Output = Result<P>>,
    load_session: impl FnOnce(&str) -> Result<Option<Session>>,
    close_shared_session: impl FnOnce(&Session, bool) -> Result<()>,
    close_worktree_session: impl FnOnce() -> Result<()>,
    remove_worktree: impl FnOnce(P) -> F,
) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    child::ensure_target_session(Some(context.session), context.parent_session_id)?;

    if close_child_session_if_sharing_worktree(
        context.session,
        context.force,
        context.parent_session_id,
        context.worktree_root,
        load_session,
        close_shared_session,
    )? {
        return Ok(());
    }

    close_with_worktree_plan(prepare_worktree, close_worktree_session, remove_worktree).await
}

fn close_session_value_without_worktree(
    session: &Session,
    args: &CloseArgs,
    child_parent_session_id: Option<&str>,
    close_session: impl FnOnce(&Session, bool) -> Result<()>,
) -> Result<()> {
    child::ensure_target_session(Some(session), child_parent_session_id)?;
    close_session(session, args.force)
}

fn close_child_session_if_sharing_worktree(
    session: &Session,
    force: bool,
    child_parent_session_id: Option<&str>,
    worktree_root: &Path,
    load_session: impl FnOnce(&str) -> Result<Option<Session>>,
    close_session: impl FnOnce(&Session, bool) -> Result<()>,
) -> Result<bool> {
    let Some(parent_session_id) = child_parent_session_id else {
        return Ok(false);
    };
    if !caller_session_is_in_worktree(parent_session_id, worktree_root, load_session)? {
        return Ok(false);
    }
    close_session(session, force)?;
    Ok(true)
}

fn caller_session_is_in_worktree(
    caller_session_id: &str,
    worktree_root: &Path,
    load_session: impl FnOnce(&str) -> Result<Option<Session>>,
) -> Result<bool> {
    Ok(match load_session(caller_session_id)? {
        Some(session) => path_is_within_worktree(&session.cwd, worktree_root),
        None => true,
    })
}

async fn close_worktree(
    worktree_root: &Path,
    args: &CloseArgs,
    refuse_current_session: bool,
    child_parent_session_id: Option<&str>,
) -> Result<()> {
    let sessions = super::store::list_sessions()?;
    let session = select_worktree_session(&sessions, worktree_root)?;
    let caller_pane_id = if session.is_some() {
        crate::shared::caller_pane::resolve_caller_pane_id()
    } else {
        None
    };
    let caller_session_id = if refuse_current_session {
        EnvVars::load().own_session_id()
    } else {
        None
    };
    let context = WorktreeCloseContext {
        args,
        session,
        child_parent_session_id,
        caller_session_id: caller_session_id.as_deref(),
        caller_pane_id: caller_pane_id.as_deref(),
    };
    let skip_hooks = args.skip_hooks;
    close_worktree_with(
        context,
        refuse_current_session,
        worktree_delete::prepare(worktree_root, args.force),
        close_worktree_session,
        |plan| async move { worktree_delete::execute(plan, skip_hooks).await },
    )
    .await
}

pub(crate) async fn close_worktree_path(worktree_path: &Path) -> Result<()> {
    let args = CloseArgs::default();
    close_worktree(worktree_path, &args, true, None).await
}

struct WorktreeCloseContext<'a> {
    args: &'a CloseArgs,
    session: Option<&'a Session>,
    child_parent_session_id: Option<&'a str>,
    caller_session_id: Option<&'a str>,
    caller_pane_id: Option<&'a str>,
}

async fn close_worktree_with<P, F>(
    context: WorktreeCloseContext<'_>,
    refuse_current_session: bool,
    prepare: impl Future<Output = Result<P>>,
    close_session: impl FnOnce(&Session, bool, Option<&str>) -> Result<()>,
    remove_worktree: impl FnOnce(P) -> F,
) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    child::ensure_target_session(context.session, context.child_parent_session_id)?;

    if refuse_current_session {
        ensure_not_closing_current_session(
            context.session,
            context.caller_session_id,
            context.caller_pane_id,
        )?;
    }
    close_with_worktree_plan(
        prepare,
        || {
            if let Some(session) = context.session {
                close_session(session, context.args.force, context.caller_pane_id)?;
            }
            Ok(())
        },
        remove_worktree,
    )
    .await
}

fn ensure_not_closing_current_session(
    session: Option<&Session>,
    caller_session_id: Option<&str>,
    current_pane_id: Option<&str>,
) -> Result<()> {
    if session.is_some_and(|session| {
        caller_session_id == Some(session.session_id.as_str())
            || is_caller_pane_session(session, current_pane_id)
    }) {
        bail!("The target worktree contains the caller's current session; skipping close");
    }
    Ok(())
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

fn close_worktree_session(
    session: &Session,
    force: bool,
    current_pane_id: Option<&str>,
) -> Result<()> {
    if !should_gracefully_close_worktree_session(session, current_pane_id) {
        return Ok(());
    }
    close_tracked_session(session, force)
}

fn should_gracefully_close_worktree_session(
    session: &Session,
    current_pane_id: Option<&str>,
) -> bool {
    session.tmux_info.is_some() && !is_caller_pane_session(session, current_pane_id)
}

fn is_caller_pane_session(session: &Session, current_pane_id: Option<&str>) -> bool {
    session
        .tmux_info
        .as_ref()
        .is_some_and(|info| current_pane_id == Some(info.pane_id.as_str()))
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
}

fn close_session<R: CloseRuntime>(session: &Session, force: bool, runtime: &R) -> Result<()> {
    let tmux_info = session
        .tmux_info
        .as_ref()
        .context("Agent session has no tmux pane")?;
    let pane_id = tmux_info.pane_id.as_str();

    if !runtime.pane_exists(tmux_info)? {
        return Ok(());
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

    match runtime.resolve_agent_pid(tmux_info, session.engine)? {
        Some(pid) => {
            if !force {
                ensure_no_unsent_draft(runtime, tmux_info, session)?;
            }

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
        None if !matches!(session.status, SessionStatus::Ended | SessionStatus::Paused) => {
            bail!(
                "Could not find a running agent process for session `{}`; its pane was left open",
                session.session_id
            );
        }
        None => {}
    }

    if runtime.pane_exists(tmux_info)? {
        ensure_pane_session(runtime, tmux_info, &session.session_id)?;
        if session.engine == Engine::Codex && session.status != SessionStatus::Ended {
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
) -> Result<()> {
    match runtime.has_draft(tmux_info, session.engine) {
        Some(false) => Ok(()),
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

    #[fixture]
    fn session() -> Session {
        Session {
            pr_context: None,
            session_id: "session-1".to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            is_delegated: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp/test-repo"),
            transcript_path: None,
            tty: None,
            tmux_info: Some(TmuxInfo {
                session_name: "test".to_string(),
                window_name: "agent".to_string(),
                window_index: 1,
                pane_id: "%42".to_string(),
                socket_path: Some("/tmp/tmux-test/socket".to_string()),
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
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Engine::Claude,
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
        "Could not check for an unsent draft in agent session `session-1`; pass --force to close it"
    )]
    #[case::codex_draft_unknown(
        Engine::Codex,
        None,
        "Could not check for an unsent draft in agent session `session-1`; pass --force to close it"
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

    #[rstest]
    #[case::normal(false)]
    #[case::forced(true)]
    fn close_refuses_to_remove_a_stopped_session_without_a_resolved_agent(
        session: Session,
        #[case] force: bool,
    ) {
        let runtime = FakeRuntime {
            draft: None,
            pid: None,
            ..FakeRuntime::default()
        };

        let result = close_session(&session, force, &runtime);

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

    #[rstest]
    #[case::same_pane(None, Some("%42"), false, Err("The target worktree contains the caller's current session; skipping close".to_string()))]
    #[case::same_session_id_without_pane(Some("session-1"), None, true, Err("The target worktree contains the caller's current session; skipping close".to_string()))]
    #[case::different_session_and_pane(Some("session-2"), Some("%99"), false, Ok(()))]
    #[case::no_caller_context(None, None, false, Ok(()))]
    #[case::untracked_session(None, None, true, Ok(()))]
    fn refuses_to_close_the_callers_current_session(
        mut session: Session,
        #[case] caller_session_id: Option<&str>,
        #[case] current_pane_id: Option<&str>,
        #[case] without_tmux_info: bool,
        #[case] expected: std::result::Result<(), String>,
    ) {
        if without_tmux_info {
            session.tmux_info = None;
        }
        assert_eq!(
            ensure_not_closing_current_session(Some(&session), caller_session_id, current_pane_id,)
                .map_err(|error| error.to_string()),
            expected,
        );
    }

    #[rstest]
    #[case::caller_in_worktree(Some("/tmp/worktrees/feature/src"), Ok(true))]
    #[case::caller_outside_worktree(Some("/tmp/other-repo"), Ok(false))]
    #[case::caller_record_missing(None, Ok(true))]
    fn detects_when_child_worktree_is_shared_with_the_caller(
        mut session: Session,
        #[case] caller_cwd: Option<&str>,
        #[case] expected: std::result::Result<bool, String>,
    ) {
        if let Some(caller_cwd) = caller_cwd {
            session.cwd = PathBuf::from(caller_cwd);
        }
        let caller_session = caller_cwd.map(|_| session.clone());

        let actual = caller_session_is_in_worktree(
            "parent-session",
            Path::new("/tmp/worktrees/feature"),
            |_| Ok(caller_session),
        )
        .map_err(|error| error.to_string());

        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case::shared_worktree(true, vec!["close child session"])]
    #[case::separate_worktree(false, Vec::new())]
    fn closes_shared_worktree_child_without_removing_its_worktree(
        mut session: Session,
        #[case] caller_shares_worktree: bool,
        #[case] expected_calls: Vec<&str>,
    ) {
        session.ancestor_session_ids = vec!["parent-session".to_string()];
        let mut parent = session.clone();
        parent.cwd = PathBuf::from(if caller_shares_worktree {
            "/tmp/worktrees/feature/src"
        } else {
            "/tmp/other-repo"
        });
        let args = CloseArgs {
            target: Some(session.session_id.clone()),
            child: true,
            ..CloseArgs::default()
        };
        let calls = RefCell::new(Vec::new());

        let result = close_child_session_if_sharing_worktree(
            &session,
            args.force,
            Some("parent-session"),
            Path::new("/tmp/worktrees/feature"),
            |_| Ok(Some(parent)),
            |_, _| {
                calls.borrow_mut().push("close child session");
                Ok(())
            },
        );

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner()
            ),
            (Ok(caller_shares_worktree), expected_calls),
        );
    }

    #[rstest]
    #[case::direct_child(
        vec!["parent-session"],
        Ok(()),
        vec!["close session safely"],
    )]
    #[case::non_child(
        vec!["other-session"],
        Err("Agent session `session-1` is not a direct child of the calling session"),
        vec![],
    )]
    fn validates_before_closing_a_tracked_session_without_a_linked_worktree(
        mut session: Session,
        #[case] ancestors: Vec<&str>,
        #[case] expected_result: std::result::Result<(), &str>,
        #[case] expected_calls: Vec<&str>,
    ) {
        session.ancestor_session_ids = ancestors.into_iter().map(str::to_string).collect();
        let args = CloseArgs {
            target: Some(session.session_id.clone()),
            child: true,
            ..CloseArgs::default()
        };
        let calls = RefCell::new(Vec::new());

        let result = close_session_value_without_worktree(
            &session,
            &args,
            Some("parent-session"),
            |_, force| {
                calls.borrow_mut().push(if force {
                    "close session with force"
                } else {
                    "close session safely"
                });
                Ok(())
            },
        );

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner(),
            ),
            (expected_result.map_err(str::to_string), expected_calls,),
        );
    }

    #[tokio::test]
    async fn pr_merge_path_refuses_the_callers_session_before_prepare() {
        let target_session = session();
        let args = CloseArgs::default();
        let calls = RefCell::new(Vec::new());
        let context = WorktreeCloseContext {
            args: &args,
            session: Some(&target_session),
            child_parent_session_id: None,
            caller_session_id: Some("session-1"),
            caller_pane_id: None,
        };
        let result = close_worktree_with(
            context,
            true,
            async {
                calls.borrow_mut().push("prepare");
                Ok::<(), anyhow::Error>(())
            },
            |_, _, _| {
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
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner(),
            ),
            (
                Err(
                    "The target worktree contains the caller's current session; skipping close"
                        .to_string()
                ),
                vec![],
            ),
        );
    }

    #[tokio::test]
    async fn session_id_child_close_rejects_non_children_before_worktree_prepare_even_with_force() {
        let mut target_session = session();
        target_session.ancestor_session_ids = vec!["other-parent".to_string()];
        let mut caller_session = session();
        caller_session.session_id = "parent-session".to_string();
        caller_session.cwd = PathBuf::from("/tmp/caller-worktree");
        let args = CloseArgs {
            target: Some(target_session.session_id.clone()),
            child: true,
            force: true,
            ..CloseArgs::default()
        };
        let calls = RefCell::new(Vec::new());

        let result = close_session_value_with_worktree(
            SessionWorktreeContext {
                session: &target_session,
                force: args.force,
                parent_session_id: Some("parent-session"),
                worktree_root: Path::new("/tmp/worktrees/feature"),
            },
            async {
                calls.borrow_mut().push("prepare worktree");
                Ok::<(), anyhow::Error>(())
            },
            |_| {
                calls.borrow_mut().push("load caller session");
                Ok(Some(caller_session))
            },
            |_, _| {
                calls.borrow_mut().push("close shared session");
                Ok(())
            },
            || {
                calls.borrow_mut().push("close worktree session");
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree");
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner(),
            ),
            (
                Err(
                    "Agent session `session-1` is not a direct child of the calling session"
                        .to_string()
                ),
                vec![],
            ),
        );
    }

    #[tokio::test]
    async fn child_close_rejects_non_children_before_prepare_even_with_force() {
        let mut target_session = session();
        target_session.ancestor_session_ids = vec!["other-parent".to_string()];
        let args = CloseArgs {
            target: Some("session-1".to_string()),
            child: true,
            force: true,
            skip_hooks: false,
        };
        let calls = RefCell::new(Vec::new());
        let context = WorktreeCloseContext {
            args: &args,
            session: Some(&target_session),
            child_parent_session_id: Some("parent-session"),
            caller_session_id: None,
            caller_pane_id: None,
        };

        let result = close_worktree_with(
            context,
            false,
            async {
                calls.borrow_mut().push("prepare");
                Ok::<(), anyhow::Error>(())
            },
            |_, _, _| {
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
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner()
            ),
            (
                Err(
                    "Agent session `session-1` is not a direct child of the calling session"
                        .to_string()
                ),
                vec![],
            ),
        );
    }

    #[tokio::test]
    async fn child_worktree_close_without_a_tracked_session_refuses_before_prepare() {
        let args = CloseArgs {
            target: Some("target-worktree".to_string()),
            child: true,
            force: false,
            skip_hooks: false,
        };
        let calls = RefCell::new(Vec::new());
        let context = WorktreeCloseContext {
            args: &args,
            session: None,
            child_parent_session_id: Some("parent-session"),
            caller_session_id: None,
            caller_pane_id: None,
        };

        let result = close_worktree_with(
            context,
            false,
            async {
                calls.borrow_mut().push("prepare");
                Ok::<(), anyhow::Error>(())
            },
            |_, _, _| {
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
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner()
            ),
            (
                Err(
                    "The target worktree has no tracked agent session; cannot verify --child"
                        .to_string()
                ),
                vec![],
            ),
        );
    }

    #[tokio::test]
    async fn child_close_allows_a_direct_child_with_force() {
        let mut target_session = session();
        target_session.ancestor_session_ids = vec!["parent-session".to_string()];
        let args = CloseArgs {
            target: Some("session-1".to_string()),
            child: true,
            force: true,
            skip_hooks: false,
        };
        let calls = RefCell::new(Vec::new());
        let context = WorktreeCloseContext {
            args: &args,
            session: Some(&target_session),
            child_parent_session_id: Some("parent-session"),
            caller_session_id: None,
            caller_pane_id: None,
        };

        let result = close_worktree_with(
            context,
            false,
            async {
                calls.borrow_mut().push("prepare");
                Ok::<(), anyhow::Error>(())
            },
            |_, force, _| {
                calls.borrow_mut().push(if force {
                    "close session with force"
                } else {
                    "close session safely"
                });
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
            (
                true,
                vec!["prepare", "close session with force", "remove worktree"],
            ),
        );
    }

    #[tokio::test]
    async fn pr_merge_close_refuses_to_close_other_sessions_without_force() {
        let target_session = session();
        let args = CloseArgs::default();
        let calls = RefCell::new(Vec::new());
        let context = WorktreeCloseContext {
            args: &args,
            session: Some(&target_session),
            child_parent_session_id: None,
            caller_session_id: Some("caller-session"),
            caller_pane_id: None,
        };
        let result = close_worktree_with(
            context,
            true,
            async {
                calls.borrow_mut().push("prepare");
                Ok::<(), anyhow::Error>(())
            },
            |_, force, _| {
                calls.borrow_mut().push(if force {
                    "close session with force"
                } else {
                    "close session safely"
                });
                anyhow::bail!("running session refused to close")
            },
            |_| async {
                calls.borrow_mut().push("remove worktree");
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner(),
            ),
            (
                Err("running session refused to close".to_string()),
                vec!["prepare", "close session safely"],
            ),
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
