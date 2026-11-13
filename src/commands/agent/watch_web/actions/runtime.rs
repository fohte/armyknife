use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use super::super::super::types::{Engine, Session, TmuxInfo};
use super::ActionRuntime;

pub(super) struct LiveActionRuntime;

impl ActionRuntime for LiveActionRuntime {
    fn send_message(&self, session: &Session, content: &str) -> Result<()> {
        super::messages::send_live_message(session, content)
    }

    fn rewind_session(
        &self,
        sessions_dir: &std::path::Path,
        session: &Session,
        message_id: &str,
        mode: super::super::rewind::RewindMode,
    ) -> Result<String> {
        super::super::rewind::rewind_session(
            sessions_dir,
            session,
            message_id,
            mode,
            &super::super::rewind::LiveHost,
        )
        .map(|session| session.session_id)
    }

    fn pane_current_command(&self, tmux_info: &TmuxInfo) -> Option<String> {
        crate::infra::tmux::get_pane_current_command_for_socket(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
        )
    }

    fn send_keys_if_agent_is_running(
        &self,
        tmux_info: &TmuxInfo,
        engine: Engine,
        current_command: &str,
        keys: &[&str],
    ) -> Result<bool> {
        let Some(pane_pid) = crate::infra::tmux::get_pane_pid_for_socket(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
        )
        .ok()
        .flatten() else {
            return Ok(false);
        };
        let Some(process_snapshot) = crate::infra::process::ProcessSnapshot::capture() else {
            return Ok(false);
        };
        if !super::super::super::pane::process::pane_has_live_agent_process(
            pane_pid,
            engine,
            Some(&process_snapshot),
        ) {
            return Ok(false);
        }

        crate::infra::tmux::send_keys_if_pane_command_matches_for_socket(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
            current_command,
            keys,
        )
        .map_err(anyhow::Error::from)
    }

    fn focus(&self, tmux_info: &TmuxInfo) -> Result<()> {
        crate::infra::tmux::focus_pane_for_socket(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
        )
        .map_err(anyhow::Error::from)
    }

    fn close_session<'a>(
        &'a self,
        session: &'a Session,
        keep_worktree: bool,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let session = session.clone();
        Box::pin(super::close::run_on_blocking_worker(move || async move {
            crate::commands::agent::close_from_watch_web(&session, keep_worktree).await
        }))
    }

    fn merge_pull_request<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
        expected_head_sha: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<crate::commands::gh::WatchWebPrMergeResult>> + Send + 'a>>
    {
        Box::pin(crate::commands::gh::run_watch_web_pr_merge(
            owner,
            repo,
            pr_number,
            expected_head_sha,
        ))
    }
}
