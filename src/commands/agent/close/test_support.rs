use std::cell::RefCell;
use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;

use anyhow::{Result, bail};
use chrono::Utc;
use rstest::fixture;

use super::super::types::{Engine, Session, SessionStatus, TmuxInfo};
use super::CloseRuntime;

#[fixture]
pub(super) fn session() -> Session {
    Session {
        archived_at: None,
        pr_context: None,
        session_id: "session-1".to_string(),
        work_type: None,
        work_type_pinned: false,
        vibe_enabled: false,
        launched_with_vibe: false,
        child_launch: Default::default(),
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
        model: None,
        effort: None,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Call {
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

pub(super) struct FakeRuntime {
    pub(super) calls: RefCell<Vec<Call>>,
    pub(super) pane_exists: Result<bool>,
    pub(super) pane_session_id: Option<String>,
    pub(super) draft: Option<bool>,
    pub(super) turn_status: std::result::Result<Option<String>, String>,
    pub(super) pid: Option<u32>,
    pub(super) pid_error: Option<String>,
    pub(super) graceful_result: io::Result<bool>,
    pub(super) signal_result: io::Result<()>,
    pub(super) wait_for_exit: bool,
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

fn test_socket_path() -> Option<String> {
    Some("/tmp/tmux-test/socket".to_string())
}

pub(super) fn pane_exists_call() -> Call {
    Call::PaneExists(test_socket_path(), "%42".to_string())
}

pub(super) fn resolve_pid_call(engine: Engine) -> Call {
    Call::ResolvePid(test_socket_path(), "%42".to_string(), engine)
}

pub(super) fn pane_session_call() -> Call {
    Call::PaneSession(test_socket_path(), "%42".to_string())
}

pub(super) fn draft_call(engine: Engine) -> Call {
    Call::Draft(test_socket_path(), "%42".to_string(), engine)
}

pub(super) fn graceful_quit_call(pid: u32) -> Call {
    Call::GracefulQuit(test_socket_path(), "%42".to_string(), pid)
}

pub(super) fn kill_pane_call() -> Call {
    Call::KillPane(test_socket_path(), "%42".to_string())
}
