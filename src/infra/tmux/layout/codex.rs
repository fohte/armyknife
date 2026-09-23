use std::fs::File;
use std::path::Path;

use anyhow::Context;

use super::AgentLaunchRoute;
use super::prompt::{
    apply_prompt_if_agent, clear_managed_codex_launch_env, wrap_in_interactive_shell,
};
use crate::commands::agent::codex_steer;
use crate::commands::agent::types::{Engine, ReasoningEffort, TMUX_SESSION_OPTION};
use crate::infra::tmux;

trait AppServerClient {
    fn wait_for_thread_started(&mut self, cwd: &Path) -> anyhow::Result<String>;
    fn start_turn(
        &mut self,
        thread_id: &str,
        prompt: &str,
        effort: Option<ReasoningEffort>,
    ) -> codex_steer::Result<()>;
}

impl AppServerClient for codex_steer::Client {
    fn wait_for_thread_started(&mut self, cwd: &Path) -> anyhow::Result<String> {
        self.wait_for_thread_started(cwd)
    }

    fn start_turn(
        &mut self,
        thread_id: &str,
        prompt: &str,
        effort: Option<ReasoningEffort>,
    ) -> codex_steer::Result<()> {
        self.start_turn(thread_id, prompt, effort)
    }
}

pub(super) struct Launch {
    state: LaunchState,
}

enum LaunchState {
    Standard,
    Connected {
        client: Box<dyn AppServerClient>,
        prompt: String,
        _lock: Option<File>,
    },
    Fallback(String),
}

pub(super) struct RecoverySpec<'a> {
    pub cwd: &'a Path,
    pub effort: Option<ReasoningEffort>,
    pub prompt_file: Option<&'a Path>,
    pub command: &'a str,
    pub model: Option<&'a str>,
    pub pane_id: Option<&'a str>,
    pub env_vars: &'a [(&'a str, &'a str)],
}

impl Launch {
    pub(super) fn prepare(
        engine: Engine,
        prompt: Option<&str>,
        pane_count: usize,
        cwd: &Path,
    ) -> Self {
        Self::prepare_with(engine, prompt, pane_count, || {
            let launch_lock = codex_steer::acquire_launch_lock(cwd)?;
            let client = codex_steer::Client::connect()?;
            Ok((Box::new(client), Some(launch_lock)))
        })
    }

    fn prepare_with(
        engine: Engine,
        prompt: Option<&str>,
        pane_count: usize,
        connect: impl FnOnce() -> anyhow::Result<(Box<dyn AppServerClient>, Option<File>)>,
    ) -> Self {
        let state = if engine != Engine::Codex || prompt.is_none() {
            LaunchState::Standard
        } else if pane_count != 1 {
            LaunchState::Fallback(format!(
                "daemon launch requires exactly one Codex pane, found {pane_count}"
            ))
        } else {
            match connect() {
                Ok((client, launch_lock)) => LaunchState::Connected {
                    client,
                    prompt: prompt.unwrap_or_default().to_string(),
                    _lock: launch_lock,
                },
                Err(error) => LaunchState::Fallback(error.to_string()),
            }
        };
        Self { state }
    }

    pub(super) fn uses_daemon(&self) -> bool {
        matches!(self.state, LaunchState::Connected { .. })
    }

    pub(super) fn command_options<'a>(
        &self,
        effort: Option<ReasoningEffort>,
        prompt_file: Option<&'a Path>,
    ) -> (Option<ReasoningEffort>, Option<&'a Path>) {
        if self.uses_daemon() {
            (None, None)
        } else {
            (effort, prompt_file)
        }
    }

    pub(super) fn finish_and_recover(
        self,
        spec: RecoverySpec<'_>,
    ) -> anyhow::Result<AgentLaunchRoute> {
        let daemon_launch = self.uses_daemon();
        let route = self.finish(spec.cwd, spec.effort);
        match route {
            AgentLaunchRoute::CodexDaemon { thread_id } => {
                let pane_id = spec
                    .pane_id
                    .context("Codex daemon launch has no pane target")?;
                tmux::set_pane_option(pane_id, TMUX_SESSION_OPTION, &thread_id).with_context(
                    || format!("Failed to bind Codex session {thread_id} to pane {pane_id}"),
                )?;
                if let Some(path) = spec.prompt_file {
                    std::fs::remove_file(path).with_context(|| {
                        format!("Failed to remove prompt file {}", path.display())
                    })?;
                }
                Ok(AgentLaunchRoute::CodexDaemon { thread_id })
            }
            AgentLaunchRoute::CodexArgvFallback { reason } if daemon_launch => {
                let pane_id = spec
                    .pane_id
                    .context("Codex argv fallback has no pane target")?;
                let command = apply_prompt_if_agent(
                    spec.command,
                    Engine::Codex,
                    spec.model,
                    spec.effort,
                    spec.prompt_file,
                    true,
                );
                let command =
                    clear_managed_codex_launch_env(&command, Engine::Codex, daemon_launch);
                let wrapped = wrap_in_interactive_shell(&command)?;
                tmux::respawn_pane_with_env(pane_id, &wrapped, spec.env_vars).with_context(
                    || format!("Codex daemon launch failed ({reason}); argv fallback also failed"),
                )?;
                Ok(AgentLaunchRoute::CodexArgvFallback { reason })
            }
            route => Ok(route),
        }
    }

    fn finish(self, cwd: &Path, effort: Option<ReasoningEffort>) -> AgentLaunchRoute {
        match self.state {
            LaunchState::Standard => AgentLaunchRoute::Standard,
            LaunchState::Fallback(reason) => AgentLaunchRoute::CodexArgvFallback { reason },
            LaunchState::Connected {
                mut client,
                prompt,
                _lock,
            } => {
                let result: anyhow::Result<String> = (|| {
                    let thread_id = client.wait_for_thread_started(cwd)?;
                    client
                        .start_turn(&thread_id, &prompt, effort)
                        .map_err(anyhow::Error::new)?;
                    Ok(thread_id)
                })();
                match result {
                    Ok(thread_id) => AgentLaunchRoute::CodexDaemon { thread_id },
                    Err(error) => AgentLaunchRoute::CodexArgvFallback {
                        reason: error.to_string(),
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    struct StubClient {
        thread: anyhow::Result<String>,
        turn: Option<codex_steer::Result<()>>,
    }

    impl AppServerClient for StubClient {
        fn wait_for_thread_started(&mut self, _cwd: &Path) -> anyhow::Result<String> {
            std::mem::replace(&mut self.thread, Ok(String::new()))
        }

        fn start_turn(
            &mut self,
            _thread_id: &str,
            _prompt: &str,
            _effort: Option<ReasoningEffort>,
        ) -> codex_steer::Result<()> {
            self.turn.take().unwrap_or(Ok(()))
        }
    }

    #[rstest]
    #[case::non_codex(Engine::Claude, Some("prompt"), 1, AgentLaunchRoute::Standard)]
    #[case::missing_prompt(Engine::Codex, None, 1, AgentLaunchRoute::Standard)]
    #[case::multiple_panes(
        Engine::Codex,
        Some("prompt"),
        2,
        AgentLaunchRoute::CodexArgvFallback {
            reason: "daemon launch requires exactly one Codex pane, found 2".to_string(),
        }
    )]
    fn preflight_routes(
        #[case] engine: Engine,
        #[case] prompt: Option<&str>,
        #[case] pane_count: usize,
        #[case] expected: AgentLaunchRoute,
    ) {
        let launch = Launch::prepare_with(engine, prompt, pane_count, || {
            Err(anyhow::anyhow!("connect should not run"))
        });

        assert_eq!(
            launch.finish(Path::new("/workspace/project-a"), None),
            expected
        );
    }

    #[rstest]
    #[case::success(
        StubClient { thread: Ok("thread-a".to_string()), turn: Some(Ok(())) },
        AgentLaunchRoute::CodexDaemon {
            thread_id: "thread-a".to_string(),
        }
    )]
    #[case::notification_failure(
        StubClient { thread: Err(anyhow::anyhow!("notification unavailable")), turn: None },
        AgentLaunchRoute::CodexArgvFallback { reason: "notification unavailable".to_string() }
    )]
    #[case::turn_rejected(
        StubClient {
            thread: Ok("thread-a".to_string()),
            turn: Some(Err(codex_steer::DeliveryError::NotDelivered(anyhow::anyhow!("rejected")))),
        },
        AgentLaunchRoute::CodexArgvFallback { reason: "rejected".to_string() }
    )]
    fn connected_routes(#[case] client: StubClient, #[case] expected: AgentLaunchRoute) {
        let launch = Launch::prepare_with(Engine::Codex, Some("prompt"), 1, || {
            Ok((Box::new(client), None))
        });

        assert_eq!(
            launch.finish(
                Path::new("/workspace/project-a"),
                Some(ReasoningEffort::Low)
            ),
            expected,
        );
    }
}
