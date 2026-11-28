use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use super::super::super::types::{Engine, Session, TmuxInfo};
use super::{ActionRuntime, CommandResult};
use crate::shared::config::Config;

pub(super) struct LiveActionRuntime;

impl ActionRuntime for LiveActionRuntime {
    fn resume_session(&self, session: &Session) -> Result<Option<String>> {
        match session.status {
            crate::commands::agent::types::SessionStatus::Paused => {
                crate::commands::agent::resume::respawn_paused_session(session)?;
                Ok(None)
            }
            crate::commands::agent::types::SessionStatus::Ended => {
                let config = crate::shared::config::load_config_or_default();
                resume_ended_session_with_config(
                    session,
                    &config,
                    &super::resume::TmuxEndedSessionHost,
                )
            }
            _ => anyhow::bail!("session is not ended or paused"),
        }
    }

    fn execute_command(&self, cwd: &std::path::Path, command: &str) -> Result<CommandResult> {
        super::bang_commands::execute_command(cwd, command)
    }

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

    fn capture_pane(&self, tmux_info: &TmuxInfo) -> Result<String> {
        crate::infra::tmux::capture_pane_for_socket_result(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
        )
        .map_err(anyhow::Error::from)
    }

    fn send_claude_escape_if_agent_is_running(
        &self,
        tmux_info: &TmuxInfo,
        current_command: &str,
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
            Engine::Claude,
            Some(&process_snapshot),
        ) {
            return Ok(false);
        }

        crate::infra::tmux::send_keys_if_pane_command_matches_for_socket(
            tmux_info.socket_path.as_deref(),
            &tmux_info.pane_id,
            current_command,
            &["Escape"],
        )
        .map_err(anyhow::Error::from)
    }

    fn interrupt_codex(&self, thread_id: &str) -> Result<()> {
        crate::commands::agent::codex_steer::interrupt_turn(thread_id)
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
        stop_and_close: bool,
        discard: Option<crate::shared::worktree_delete::discard::ConfirmedDiscard>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let session = session.clone();
        Box::pin(super::close::run_on_blocking_worker(move || async move {
            crate::commands::agent::close_from_watch_web(
                &session,
                keep_worktree,
                stop_and_close,
                discard,
            )
            .await
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

fn resume_ended_session_with_config(
    session: &Session,
    config: &Config,
    host: &impl super::resume::EndedSessionHost,
) -> Result<Option<String>> {
    super::resume::resume_ended_session(session, &config.agent.worktree.layout, host)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use anyhow::Result;
    use serde_json::{Value, json};

    use super::resume_ended_session_with_config;
    use crate::commands::agent::types::{Engine, Session, TmuxInfo};
    use crate::infra::tmux::layout::ResumeLayoutSpec;
    use crate::shared::config::{Config, LayoutNode, PaneConfig};

    #[derive(Default)]
    struct FakeEndedSessionHost {
        calls: Mutex<Vec<Value>>,
    }

    impl super::super::resume::EndedSessionHost for FakeEndedSessionHost {
        fn open_layout(&self, spec: ResumeLayoutSpec<'_>) -> Result<Option<String>> {
            self.calls.lock().unwrap().push(json!({
                "layout": serde_json::to_value(spec.layout).unwrap(),
                "agentCommand": spec.agent_command,
            }));
            Ok(Some("%18".to_string()))
        }
    }

    #[test]
    fn ended_resume_uses_the_worktree_layout_from_config() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.agent.worktree.layout = LayoutNode::Pane(PaneConfig {
            command: "fixture-editor".to_string(),
            focus: true,
        });
        let mut session: Session = serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/worktree",
            "transcript_path": null,
            "tmux_info": null,
            "status": "ended",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .unwrap();
        session.cwd = temp_dir.path().to_path_buf();
        session.engine = Engine::Codex;
        session.tmux_info = Some(TmuxInfo {
            session_name: "fixture-session-window".to_string(),
            window_name: "fixture-agent".to_string(),
            window_index: 2,
            pane_id: "%17".to_string(),
            socket_path: Some("/fixture/tmux.sock".to_string()),
        });
        let host = FakeEndedSessionHost::default();

        resume_ended_session_with_config(&session, &config, &host).unwrap();

        assert_eq!(
            *host.calls.lock().unwrap(),
            vec![json!({
                "layout": { "command": "fixture-editor", "focus": true },
                "agentCommand": "a agent resume fixture-session --engine codex",
            })],
        );
    }
}
