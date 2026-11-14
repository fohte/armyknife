use std::fs::File;
use std::path::Path;

use anyhow::{Context, bail};

use super::AgentLaunchRoute;
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
}

pub(super) struct RecoverySpec<'a> {
    pub cwd: &'a Path,
    pub effort: Option<ReasoningEffort>,
    pub prompt_file: Option<&'a Path>,
    pub pane_id: Option<&'a str>,
    pub initial_prompt_marker: Option<super::InitialPromptMarker<'a>>,
}

impl Launch {
    pub(super) fn prepare(
        engine: Engine,
        prompt: Option<&str>,
        pane_count: usize,
        cwd: &Path,
    ) -> anyhow::Result<Self> {
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
    ) -> anyhow::Result<Self> {
        let state = if engine != Engine::Codex || prompt.is_none() {
            LaunchState::Standard
        } else if pane_count != 1 {
            bail!("Codex daemon launch requires exactly one Codex pane, found {pane_count}");
        } else {
            let (client, launch_lock) =
                connect().context("Failed to connect to Codex app-server")?;
            LaunchState::Connected {
                client,
                prompt: prompt.unwrap_or_default().to_string(),
                _lock: launch_lock,
            }
        };
        Ok(Self { state })
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
        self.finish_and_recover_with(spec, |pane_id, thread_id| {
            tmux::set_pane_option(pane_id, TMUX_SESSION_OPTION, thread_id)
                .map_err(anyhow::Error::new)
        })
    }

    fn finish_and_recover_with(
        self,
        spec: RecoverySpec<'_>,
        bind_pane: impl FnOnce(&str, &str) -> anyhow::Result<()>,
    ) -> anyhow::Result<AgentLaunchRoute> {
        let route =
            self.finish_with_marker(spec.cwd, spec.effort, spec.initial_prompt_marker)
                .with_context(|| {
                    spec.pane_id.map_or_else(
                || "Codex initial prompt delivery failed after creating a pane".to_string(),
                |pane_id| {
                    format!("Codex pane {pane_id} was created, but initial prompt delivery failed")
                },
            )
                })?;
        let Some(thread_id) = route.codex_thread_id() else {
            return Ok(route);
        };
        let pane_id = spec
            .pane_id
            .context("Codex daemon launch has no pane target")?;
        bind_pane(pane_id, thread_id).with_context(|| {
            format!("Failed to bind Codex session {thread_id} to pane {pane_id}")
        })?;
        if let Some(path) = spec.prompt_file {
            std::fs::remove_file(path)
                .with_context(|| format!("Failed to remove prompt file {}", path.display()))?;
        }
        Ok(route)
    }

    #[cfg(test)]
    fn finish(
        self,
        cwd: &Path,
        effort: Option<ReasoningEffort>,
    ) -> anyhow::Result<AgentLaunchRoute> {
        self.finish_with_marker(cwd, effort, None)
    }

    fn finish_with_marker(
        self,
        cwd: &Path,
        effort: Option<ReasoningEffort>,
        initial_prompt_marker: Option<super::InitialPromptMarker<'_>>,
    ) -> anyhow::Result<AgentLaunchRoute> {
        match self.state {
            LaunchState::Standard => Ok(AgentLaunchRoute::Standard),
            LaunchState::Connected {
                mut client,
                prompt,
                _lock,
            } => {
                let thread_id = client
                    .wait_for_thread_started(cwd)
                    .context("Failed to wait for Codex app-server thread/start")?;
                if let Some(marker) = initial_prompt_marker
                    && let Err(error) = (marker.mark_pending)(&thread_id)
                {
                    tracing::warn!(
                        event = "agent.new.initial_prompt_marker_write_failed",
                        session_id = %thread_id,
                        error = %error,
                    );
                }
                match client.start_turn(&thread_id, &prompt, effort) {
                    Ok(()) => Ok(AgentLaunchRoute::CodexDaemon { thread_id }),
                    Err(codex_steer::DeliveryError::Unconfirmed(error)) => {
                        Ok(AgentLaunchRoute::CodexDaemonTurnUnconfirmed {
                            thread_id,
                            reason: error.to_string(),
                        })
                    }
                    Err(codex_steer::DeliveryError::NotDelivered(error)) => {
                        if let Some(marker) = initial_prompt_marker
                            && let Err(clear_error) = (marker.clear_pending)(&thread_id)
                        {
                            tracing::warn!(
                                event = "agent.new.initial_prompt_marker_cleanup_failed",
                                session_id = %thread_id,
                                error = %clear_error,
                            );
                        }
                        Err(error)
                            .context("Codex app-server rejected the initial turn/start request")
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use super::*;
    use rstest::{fixture, rstest};

    #[fixture]
    fn prompt_file() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prompt.txt");
        std::fs::write(&path, "prompt").unwrap();
        (dir, path)
    }

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

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Wait,
        Mark(String),
        StartTurn(String, String),
    }

    struct OrderedClient {
        calls: Arc<Mutex<Vec<Call>>>,
    }

    impl AppServerClient for OrderedClient {
        fn wait_for_thread_started(&mut self, _cwd: &Path) -> anyhow::Result<String> {
            self.calls.lock().unwrap().push(Call::Wait);
            Ok("thread-example".to_string())
        }

        fn start_turn(
            &mut self,
            thread_id: &str,
            prompt: &str,
            _effort: Option<ReasoningEffort>,
        ) -> codex_steer::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::StartTurn(thread_id.to_string(), prompt.to_string()));
            Ok(())
        }
    }

    #[test]
    fn marks_initial_prompt_before_starting_the_turn() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mark_calls = calls.clone();
        let mark_pending = move |session_id: &str| {
            mark_calls
                .lock()
                .unwrap()
                .push(Call::Mark(session_id.to_string()));
            Ok(())
        };
        let clear_pending = |_session_id: &str| Ok(());
        let launch = Launch::prepare_with(Engine::Codex, Some("prompt"), 1, || {
            Ok((
                Box::new(OrderedClient {
                    calls: calls.clone(),
                }),
                None,
            ))
        })
        .expect("app-server client should be created");

        let route = launch.finish_and_recover_with(
            RecoverySpec {
                cwd: Path::new("/workspace/project-a"),
                effort: None,
                prompt_file: None,
                pane_id: Some("%42"),
                initial_prompt_marker: Some(super::super::InitialPromptMarker {
                    mark_pending: &mark_pending,
                    clear_pending: &clear_pending,
                }),
            },
            |_, _| Ok(()),
        );

        assert_eq!(
            (
                route.map_err(|error| error.to_string()),
                calls.lock().unwrap().clone(),
            ),
            (
                Ok(AgentLaunchRoute::CodexDaemon {
                    thread_id: "thread-example".to_string(),
                }),
                vec![
                    Call::Wait,
                    Call::Mark("thread-example".to_string()),
                    Call::StartTurn("thread-example".to_string(), "prompt".to_string()),
                ],
            ),
        );
    }

    #[test]
    fn initial_prompt_marker_write_failure_does_not_block_starting_the_turn() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mark_pending = |_session_id: &str| Err(anyhow::anyhow!("database unavailable"));
        let clear_pending = |_session_id: &str| Ok(());
        let launch = Launch::prepare_with(Engine::Codex, Some("prompt"), 1, || {
            Ok((
                Box::new(OrderedClient {
                    calls: calls.clone(),
                }),
                None,
            ))
        })
        .expect("app-server client should be created");

        let route = launch.finish_and_recover_with(
            RecoverySpec {
                cwd: Path::new("/workspace/project-a"),
                effort: None,
                prompt_file: None,
                pane_id: Some("%42"),
                initial_prompt_marker: Some(super::super::InitialPromptMarker {
                    mark_pending: &mark_pending,
                    clear_pending: &clear_pending,
                }),
            },
            |_, _| Ok(()),
        );

        assert_eq!(
            (
                route.map_err(|error| error.to_string()),
                calls.lock().unwrap().clone(),
            ),
            (
                Ok(AgentLaunchRoute::CodexDaemon {
                    thread_id: "thread-example".to_string(),
                }),
                vec![
                    Call::Wait,
                    Call::StartTurn("thread-example".to_string(), "prompt".to_string()),
                ],
            ),
        );
    }

    #[test]
    fn rejected_turn_clears_the_pending_initial_prompt_marker() {
        let cleared_ids = Arc::new(Mutex::new(Vec::new()));
        let clear_ids = cleared_ids.clone();
        let mark_pending = |_session_id: &str| Ok(());
        let clear_pending = move |session_id: &str| {
            clear_ids.lock().unwrap().push(session_id.to_string());
            Ok(())
        };
        let launch = Launch::prepare_with(Engine::Codex, Some("prompt"), 1, || {
            Ok((
                Box::new(StubClient {
                    thread: Ok("thread-example".to_string()),
                    turn: Some(Err(codex_steer::DeliveryError::NotDelivered(
                        anyhow::anyhow!("rejected"),
                    ))),
                }),
                None,
            ))
        })
        .expect("app-server client should be created");

        let route = launch.finish_and_recover_with(
            RecoverySpec {
                cwd: Path::new("/workspace/project-a"),
                effort: None,
                prompt_file: None,
                pane_id: Some("%42"),
                initial_prompt_marker: Some(super::super::InitialPromptMarker {
                    mark_pending: &mark_pending,
                    clear_pending: &clear_pending,
                }),
            },
            |_, _| panic!("a rejected turn must not bind the pane"),
        );

        assert_eq!(
            (
                route.map_err(|error| format!("{error:#}")),
                cleared_ids.lock().unwrap().clone(),
            ),
            (
                Err("Codex pane %42 was created, but initial prompt delivery failed: Codex app-server rejected the initial turn/start request: rejected".to_string()),
                vec!["thread-example".to_string()],
            ),
        );
    }

    #[rstest]
    #[case::non_codex(Engine::Claude, Some("prompt"), 1, AgentLaunchRoute::Standard)]
    #[case::missing_prompt(Engine::Codex, None, 1, AgentLaunchRoute::Standard)]
    fn preflight_routes(
        #[case] engine: Engine,
        #[case] prompt: Option<&str>,
        #[case] pane_count: usize,
        #[case] expected: AgentLaunchRoute,
    ) {
        let launch = Launch::prepare_with(engine, prompt, pane_count, || {
            Err(anyhow::anyhow!("connect should not run"))
        })
        .expect("preflight should succeed");

        assert_eq!(
            launch
                .finish(Path::new("/workspace/project-a"), None)
                .map_err(|error| format!("{error:#}")),
            Ok(expected)
        );
    }

    #[test]
    fn multiple_panes_fail_before_connecting() {
        let result = Launch::prepare_with(Engine::Codex, Some("prompt"), 2, || {
            panic!("app-server should not be contacted")
        });

        assert_eq!(
            result.err().map(|error| error.to_string()),
            Some("Codex daemon launch requires exactly one Codex pane, found 2".to_string()),
        );
    }

    #[test]
    fn connection_failure_is_reported_before_pane_creation() {
        let result = Launch::prepare_with(Engine::Codex, Some("prompt"), 1, || {
            Err(anyhow::anyhow!("app-server unavailable"))
        });

        assert_eq!(
            result.err().map(|error| format!("{error:#}")),
            Some("Failed to connect to Codex app-server: app-server unavailable".to_string()),
        );
    }

    #[rstest]
    #[case::success(
        StubClient { thread: Ok("thread-example".to_string()), turn: Some(Ok(())) },
        Ok(AgentLaunchRoute::CodexDaemon {
            thread_id: "thread-example".to_string(),
        })
    )]
    #[case::thread_start_failure(
        StubClient { thread: Err(anyhow::anyhow!("thread wait failed")), turn: None },
        Err("Failed to wait for Codex app-server thread/start: thread wait failed".to_string())
    )]
    #[case::turn_rejected(
        StubClient {
            thread: Ok("thread-example".to_string()),
            turn: Some(Err(codex_steer::DeliveryError::NotDelivered(anyhow::anyhow!("rejected")))),
        },
        Err("Codex app-server rejected the initial turn/start request: rejected".to_string())
    )]
    #[case::turn_unconfirmed(
        StubClient {
            thread: Ok("thread-example".to_string()),
            turn: Some(Err(codex_steer::DeliveryError::Unconfirmed(anyhow::anyhow!("response unavailable")))),
        },
        Ok(AgentLaunchRoute::CodexDaemonTurnUnconfirmed {
            thread_id: "thread-example".to_string(),
            reason: "response unavailable".to_string(),
        })
    )]
    fn connected_routes(
        #[case] client: StubClient,
        #[case] expected: Result<AgentLaunchRoute, String>,
    ) {
        let launch = Launch::prepare_with(Engine::Codex, Some("prompt"), 1, || {
            Ok((Box::new(client), None))
        })
        .expect("connection should succeed");

        assert_eq!(
            launch
                .finish(
                    Path::new("/workspace/project-a"),
                    Some(ReasoningEffort::Low)
                )
                .map_err(|error| format!("{error:#}")),
            expected,
        );
    }

    #[test]
    fn unconfirmed_turn_binds_its_existing_thread_to_the_pane() {
        let launch = Launch::prepare_with(Engine::Codex, Some("prompt"), 1, || {
            Ok((
                Box::new(StubClient {
                    thread: Ok("thread-example".to_string()),
                    turn: Some(Err(codex_steer::DeliveryError::Unconfirmed(
                        anyhow::anyhow!("response unavailable"),
                    ))),
                }),
                None,
            ))
        })
        .expect("connection should succeed");
        let mut binding = None;

        let route = launch.finish_and_recover_with(
            RecoverySpec {
                cwd: Path::new("/workspace/project-a"),
                effort: None,
                prompt_file: None,
                pane_id: Some("%42"),
                initial_prompt_marker: None,
            },
            |pane_id, thread_id| {
                binding = Some((pane_id.to_string(), thread_id.to_string()));
                Ok(())
            },
        );

        assert_eq!(
            (route.map_err(|error| format!("{error:#}")), binding,),
            (
                Ok(AgentLaunchRoute::CodexDaemonTurnUnconfirmed {
                    thread_id: "thread-example".to_string(),
                    reason: "response unavailable".to_string(),
                }),
                Some(("%42".to_string(), "thread-example".to_string())),
            ),
        );
    }

    #[rstest]
    fn successful_turn_binds_its_thread_and_removes_prompt_file(
        prompt_file: (tempfile::TempDir, PathBuf),
    ) {
        let (_dir, prompt_path) = prompt_file;
        let launch = Launch::prepare_with(Engine::Codex, Some("prompt"), 1, || {
            Ok((
                Box::new(StubClient {
                    thread: Ok("thread-example".to_string()),
                    turn: Some(Ok(())),
                }),
                None,
            ))
        })
        .expect("connection should succeed");
        let mut binding = None;

        let route = launch.finish_and_recover_with(
            RecoverySpec {
                cwd: Path::new("/workspace/project-a"),
                effort: None,
                prompt_file: Some(&prompt_path),
                pane_id: Some("%42"),
                initial_prompt_marker: None,
            },
            |pane_id, thread_id| {
                binding = Some((pane_id.to_string(), thread_id.to_string()));
                Ok(())
            },
        );

        assert_eq!(
            (
                route.map_err(|error| format!("{error:#}")),
                binding,
                prompt_path.exists(),
            ),
            (
                Ok(AgentLaunchRoute::CodexDaemon {
                    thread_id: "thread-example".to_string(),
                }),
                Some(("%42".to_string(), "thread-example".to_string())),
                false,
            ),
        );
    }

    #[test]
    fn daemon_route_requires_a_pane_target() {
        let launch = Launch::prepare_with(Engine::Codex, Some("prompt"), 1, || {
            Ok((
                Box::new(StubClient {
                    thread: Ok("thread-example".to_string()),
                    turn: Some(Ok(())),
                }),
                None,
            ))
        })
        .expect("connection should succeed");
        let mut binding = None;

        let route = launch.finish_and_recover_with(
            RecoverySpec {
                cwd: Path::new("/workspace/project-a"),
                effort: None,
                prompt_file: None,
                pane_id: None,
                initial_prompt_marker: None,
            },
            |pane_id, thread_id| {
                binding = Some((pane_id.to_string(), thread_id.to_string()));
                Ok(())
            },
        );

        assert_eq!(
            (route.map_err(|error| format!("{error:#}")), binding),
            (
                Err("Codex daemon launch has no pane target".to_string()),
                None
            ),
        );
    }

    #[test]
    fn delivery_failure_identifies_the_created_pane() {
        let launch = Launch::prepare_with(Engine::Codex, Some("prompt"), 1, || {
            Ok((
                Box::new(StubClient {
                    thread: Ok("thread-example".to_string()),
                    turn: Some(Err(codex_steer::DeliveryError::NotDelivered(
                        anyhow::anyhow!("rejected"),
                    ))),
                }),
                None,
            ))
        })
        .expect("connection should succeed");

        let route = launch.finish_and_recover_with(
            RecoverySpec {
                cwd: Path::new("/workspace/project-a"),
                effort: None,
                prompt_file: None,
                pane_id: Some("%42"),
                initial_prompt_marker: None,
            },
            |_, _| panic!("a rejected turn must not bind the pane"),
        );

        assert_eq!(
            route.map_err(|error| format!("{error:#}")),
            Err(
                "Codex pane %42 was created, but initial prompt delivery failed: Codex app-server rejected the initial turn/start request: rejected".to_string()
            ),
        );
    }
}
