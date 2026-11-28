use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use super::super::super::types::{Engine, Session, TmuxInfo, resolve_session_option_with};
use super::resume::{RetryResumeError, RetryResumeOutcome};
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

    fn retry_resume(
        &self,
        session: &Session,
        check_generation: &mut dyn FnMut() -> std::result::Result<bool, RetryResumeError>,
    ) -> std::result::Result<RetryResumeOutcome, RetryResumeError> {
        retry_stuck_resume_with(
            session,
            |socket_path, pane_id, option| {
                let format = format!("#{{{option}}}");
                crate::infra::tmux::run_tmux_output_for_socket(
                    socket_path,
                    &["display-message", "-p", "-t", pane_id, format.as_str()],
                )
                .map(|value| (!value.is_empty()).then_some(value))
                .map_err(anyhow::Error::from)
            },
            check_generation,
            |session| {
                crate::commands::agent::resume::respawn_stuck_session(session)
                    .map(|_| ())
                    .map_err(anyhow::Error::from)
                    .map_err(RetryResumeError::Runtime)
            },
        )
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

fn retry_stuck_resume_with(
    session: &Session,
    mut read_pane_option: impl FnMut(Option<&str>, &str, &str) -> Result<Option<String>>,
    check_generation: &mut dyn FnMut() -> std::result::Result<bool, RetryResumeError>,
    respawn: impl FnOnce(&Session) -> std::result::Result<(), RetryResumeError>,
) -> std::result::Result<RetryResumeOutcome, RetryResumeError> {
    if let Some(tmux_info) = session.tmux_info.as_ref() {
        let socket_path = tmux_info.socket_path.as_deref();
        let recorded_session_id = resolve_session_option_with(|option| {
            read_pane_option(socket_path, &tmux_info.pane_id, option)
        })
        .map_err(RetryResumeError::Runtime)?;
        if recorded_session_id.as_deref() != Some(session.session_id.as_str()) {
            return Ok(RetryResumeOutcome::PaneMismatch);
        }
    }

    if !check_generation()? {
        return Ok(RetryResumeOutcome::StartupChanged);
    }
    respawn(session)?;
    Ok(RetryResumeOutcome::Retried)
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
    use rstest::rstest;
    use serde_json::{Value, json};

    use super::super::resume::{RetryResumeError, RetryResumeOutcome};
    use super::{resume_ended_session_with_config, retry_stuck_resume_with};
    use crate::commands::agent::types::{
        Engine, Session, TMUX_SESSION_OPTION, TMUX_SESSION_OPTION_LEGACY, TmuxInfo,
    };
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

    #[rstest]
    #[case::primary_option_matches(
        TMUX_SESSION_OPTION,
        Some("fixture-session"),
        true,
        RetryResumeOutcome::Retried,
        vec![json!({
            "socketPath": "/fixture/tmux.sock",
            "paneId": "%17",
            "option": "@armyknife-last-agent-session-id",
        })],
        vec![json!({ "sessionId": "fixture-session" })],
    )]
    #[case::legacy_option_matches(
        TMUX_SESSION_OPTION_LEGACY,
        Some("fixture-session"),
        true,
        RetryResumeOutcome::Retried,
        vec![
            json!({
                "socketPath": "/fixture/tmux.sock",
                "paneId": "%17",
                "option": "@armyknife-last-agent-session-id",
            }),
            json!({
                "socketPath": "/fixture/tmux.sock",
                "paneId": "%17",
                "option": "@armyknife-last-claude-code-session-id",
            }),
        ],
        vec![json!({ "sessionId": "fixture-session" })],
    )]
    #[case::another_session_is_recorded(
        TMUX_SESSION_OPTION,
        Some("fixture-other"),
        false,
        RetryResumeOutcome::PaneMismatch,
        vec![json!({
            "socketPath": "/fixture/tmux.sock",
            "paneId": "%17",
            "option": "@armyknife-last-agent-session-id",
        })],
        vec![],
    )]
    #[case::no_session_is_recorded(
        TMUX_SESSION_OPTION,
        None,
        false,
        RetryResumeOutcome::PaneMismatch,
        vec![
            json!({
                "socketPath": "/fixture/tmux.sock",
                "paneId": "%17",
                "option": "@armyknife-last-agent-session-id",
            }),
            json!({
                "socketPath": "/fixture/tmux.sock",
                "paneId": "%17",
                "option": "@armyknife-last-claude-code-session-id",
            }),
        ],
        vec![],
    )]
    #[case::generation_advanced_after_pane_match(
        TMUX_SESSION_OPTION,
        Some("fixture-session"),
        false,
        RetryResumeOutcome::StartupChanged,
        vec![json!({
            "socketPath": "/fixture/tmux.sock",
            "paneId": "%17",
            "option": "@armyknife-last-agent-session-id",
        })],
        vec![],
    )]
    fn retry_respawns_only_when_the_pane_records_the_requested_session(
        #[case] recorded_option: &str,
        #[case] recorded_session_id: Option<&str>,
        #[case] generation_matches: bool,
        #[case] expected_outcome: RetryResumeOutcome,
        #[case] expected_option_reads: Vec<Value>,
        #[case] expected_respawn_calls: Vec<Value>,
    ) {
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
        session.tmux_info = Some(TmuxInfo {
            session_name: "fixture-tmux-session".to_string(),
            window_name: "fixture-window".to_string(),
            window_index: 0,
            pane_id: "%17".to_string(),
            socket_path: Some("/fixture/tmux.sock".to_string()),
        });
        let mut option_reads = Vec::new();
        let mut respawn_calls = Vec::new();

        let result = retry_stuck_resume_with(
            &session,
            |socket_path, pane_id, option| {
                option_reads.push(json!({
                    "socketPath": socket_path,
                    "paneId": pane_id,
                    "option": option,
                }));
                Ok((option == recorded_option)
                    .then(|| recorded_session_id.map(str::to_string))
                    .flatten())
            },
            &mut || Ok(generation_matches),
            |session| {
                respawn_calls.push(json!({ "sessionId": session.session_id }));
                Ok(())
            },
        )
        .map_err(retry_error_message);

        assert_eq!(
            (result, option_reads, respawn_calls),
            (
                Ok(expected_outcome),
                expected_option_reads,
                expected_respawn_calls,
            ),
        );
    }

    #[test]
    fn retry_preserves_pane_option_lookup_errors() {
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
        session.tmux_info = Some(TmuxInfo {
            session_name: "fixture-tmux-session".to_string(),
            window_name: "fixture-window".to_string(),
            window_index: 0,
            pane_id: "%17".to_string(),
            socket_path: Some("/fixture/tmux.sock".to_string()),
        });
        let mut respawn_calls = Vec::new();

        let result = retry_stuck_resume_with(
            &session,
            |_, _, _| Err(anyhow::anyhow!("fixture pane option read failed")),
            &mut || Ok(true),
            |session| {
                respawn_calls.push(session.session_id.clone());
                Ok(())
            },
        )
        .map(|outcome| outcome == RetryResumeOutcome::Retried)
        .map_err(retry_error_message);

        assert_eq!(
            (result, respawn_calls),
            (
                Err("fixture pane option read failed".to_string()),
                Vec::new()
            ),
        );
    }

    fn retry_error_message(error: RetryResumeError) -> String {
        match error {
            RetryResumeError::Store(_) => "fixture store error".to_string(),
            RetryResumeError::Runtime(error) => error.to_string(),
        }
    }
}
