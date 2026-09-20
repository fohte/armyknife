use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

use super::AgentLaunchRoute;
use super::codex::wrap_in_interactive_shell;
use super::prompt::apply_prompt_if_agent;
use crate::commands::agent::claude_registry::PeerConnection;
use crate::commands::agent::types::{Engine, ReasoningEffort};
use crate::commands::agent::{claude_messaging, claude_registry};
use crate::infra::tmux;

const REGISTRY_POLL_INTERVAL: Duration = Duration::from_millis(250);
const REGISTRY_POLL_TIMEOUT: Duration = Duration::from_secs(20);

trait MessagingClient {
    fn wait_for_connection(&mut self, tmux_location: &str) -> anyhow::Result<PeerConnection>;
    fn send_message(&mut self, connection: &PeerConnection, prompt: &str) -> anyhow::Result<()>;
}

struct Client;

impl MessagingClient for Client {
    fn wait_for_connection(&mut self, tmux_location: &str) -> anyhow::Result<PeerConnection> {
        let deadline = Instant::now() + REGISTRY_POLL_TIMEOUT;
        loop {
            if let Some(connection) = claude_registry::load_peer_connection_by_tmux(tmux_location)
                && connection
                    .messaging_socket_path
                    .as_deref()
                    .is_some_and(|path| !path.is_empty())
            {
                return Ok(connection);
            }
            if Instant::now() >= deadline {
                bail!(
                    "timed out waiting for Claude messaging socket at tmux location {tmux_location}"
                );
            }
            thread::sleep(REGISTRY_POLL_INTERVAL);
        }
    }

    fn send_message(&mut self, connection: &PeerConnection, prompt: &str) -> anyhow::Result<()> {
        let socket_path = connection
            .messaging_socket_path
            .as_deref()
            .context("Claude registry entry has no messaging socket path")?;
        claude_messaging::send_message(socket_path, connection.pid, prompt)
    }
}

pub(super) struct Launch {
    state: LaunchState,
}

enum LaunchState {
    Standard,
    Messaging {
        client: Box<dyn MessagingClient>,
        prompt: String,
    },
    Fallback(String),
}

pub(super) struct RecoverySpec<'a> {
    pub effort: Option<ReasoningEffort>,
    pub prompt_file: Option<&'a Path>,
    pub command: &'a str,
    pub model: Option<&'a str>,
    pub pane_id: Option<&'a str>,
    pub tmux_location: Option<&'a str>,
    pub env_vars: &'a [(&'a str, &'a str)],
}

impl Launch {
    pub(super) fn prepare(engine: Engine, prompt: Option<&str>, pane_count: usize) -> Self {
        Self::prepare_with(engine, prompt, pane_count, || Box::new(Client))
    }

    fn prepare_with(
        engine: Engine,
        prompt: Option<&str>,
        pane_count: usize,
        client: impl FnOnce() -> Box<dyn MessagingClient>,
    ) -> Self {
        let state = if engine != Engine::Claude || prompt.is_none() {
            LaunchState::Standard
        } else if pane_count != 1 {
            LaunchState::Fallback(format!(
                "messaging launch requires exactly one Claude pane, found {pane_count}"
            ))
        } else {
            LaunchState::Messaging {
                client: client(),
                prompt: prompt.unwrap_or_default().to_string(),
            }
        };
        Self { state }
    }

    pub(super) fn uses_messaging(&self) -> bool {
        matches!(self.state, LaunchState::Messaging { .. })
    }

    pub(super) fn command_options<'a>(
        &self,
        effort: Option<ReasoningEffort>,
        prompt_file: Option<&'a Path>,
    ) -> (Option<ReasoningEffort>, Option<&'a Path>) {
        if self.uses_messaging() {
            (effort, None)
        } else {
            (effort, prompt_file)
        }
    }

    pub(super) fn finish_and_recover(
        self,
        spec: RecoverySpec<'_>,
    ) -> anyhow::Result<AgentLaunchRoute> {
        self.finish_and_recover_with(spec, |pane_id, command, env_vars| {
            tmux::respawn_pane_with_env(pane_id, command, env_vars).map_err(anyhow::Error::new)
        })
    }

    fn finish_and_recover_with(
        self,
        spec: RecoverySpec<'_>,
        respawn: impl FnOnce(&str, &str, &[(&str, &str)]) -> anyhow::Result<()>,
    ) -> anyhow::Result<AgentLaunchRoute> {
        let messaging_launch = self.uses_messaging();
        let route = self.finish(spec.tmux_location);
        match route {
            AgentLaunchRoute::ClaudeMessaging => {
                if let Some(path) = spec.prompt_file {
                    std::fs::remove_file(path).with_context(|| {
                        format!("Failed to remove prompt file {}", path.display())
                    })?;
                }
                Ok(AgentLaunchRoute::ClaudeMessaging)
            }
            AgentLaunchRoute::ClaudeArgvFallback { reason } if messaging_launch => {
                let pane_id = spec
                    .pane_id
                    .context("Claude argv fallback has no pane target")?;
                let command = apply_prompt_if_agent(
                    spec.command,
                    Engine::Claude,
                    spec.model,
                    spec.effort,
                    spec.prompt_file,
                    true,
                );
                let wrapped = wrap_in_interactive_shell(&command)?;
                respawn(pane_id, &wrapped, spec.env_vars).with_context(|| {
                    format!("Claude messaging launch failed ({reason}); argv fallback also failed")
                })?;
                Ok(AgentLaunchRoute::ClaudeArgvFallback { reason })
            }
            route => Ok(route),
        }
    }

    fn finish(self, tmux_location: Option<&str>) -> AgentLaunchRoute {
        match self.state {
            LaunchState::Standard => AgentLaunchRoute::Standard,
            LaunchState::Fallback(reason) => AgentLaunchRoute::ClaudeArgvFallback { reason },
            LaunchState::Messaging { mut client, prompt } => {
                let result = (|| {
                    let tmux_location =
                        tmux_location.context("Claude messaging launch has no tmux location")?;
                    let connection = client.wait_for_connection(tmux_location)?;
                    client.send_message(&connection, &prompt)
                })();
                match result {
                    Ok(()) => AgentLaunchRoute::ClaudeMessaging,
                    Err(error) => AgentLaunchRoute::ClaudeArgvFallback {
                        reason: error.to_string(),
                    },
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Wait(String),
        Send(PeerConnection, String),
    }

    struct StubClient {
        connection: anyhow::Result<PeerConnection>,
        delivery: Option<anyhow::Result<()>>,
        calls: Arc<Mutex<Vec<Call>>>,
    }

    impl MessagingClient for StubClient {
        fn wait_for_connection(&mut self, tmux_location: &str) -> anyhow::Result<PeerConnection> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Wait(tmux_location.to_string()));
            std::mem::replace(
                &mut self.connection,
                Err(anyhow::anyhow!("connection already consumed")),
            )
        }

        fn send_message(
            &mut self,
            connection: &PeerConnection,
            prompt: &str,
        ) -> anyhow::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Send(connection.clone(), prompt.to_string()));
            self.delivery.take().unwrap_or(Ok(()))
        }
    }

    fn connection() -> PeerConnection {
        PeerConnection {
            pid: 123,
            messaging_socket_path: Some("/tmp/cc-socks/123.sock".to_string()),
        }
    }

    #[fixture]
    fn prompt_file() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("prompt.txt");
        std::fs::write(&path, "prompt").unwrap();
        (dir, path)
    }

    #[rstest]
    #[case::non_claude(Engine::Codex, Some("prompt"), 1, AgentLaunchRoute::Standard)]
    #[case::missing_prompt(Engine::Claude, None, 1, AgentLaunchRoute::Standard)]
    #[case::multiple_panes(
        Engine::Claude,
        Some("prompt"),
        2,
        AgentLaunchRoute::ClaudeArgvFallback {
            reason: "messaging launch requires exactly one Claude pane, found 2".to_string(),
        }
    )]
    fn preflight_routes(
        #[case] engine: Engine,
        #[case] prompt: Option<&str>,
        #[case] pane_count: usize,
        #[case] expected: AgentLaunchRoute,
    ) {
        let launch = Launch::prepare_with(engine, prompt, pane_count, || {
            panic!("client should not be created")
        });

        assert_eq!(launch.finish(Some("example/repo:@1.%2")), expected);
    }

    #[test]
    fn messaging_command_options_keep_effort_and_omit_prompt_file() {
        let launch = Launch::prepare_with(Engine::Claude, Some("prompt"), 1, || {
            Box::new(StubClient {
                connection: Ok(connection()),
                delivery: Some(Ok(())),
                calls: Arc::new(Mutex::new(Vec::new())),
            })
        });
        let prompt_path = Path::new("/tmp/example-prompt.txt");

        let options = launch.command_options(Some(ReasoningEffort::Max), Some(prompt_path));

        assert_eq!(options, (Some(ReasoningEffort::Max), None));
    }

    #[rstest]
    #[case::success(
        Ok(connection()),
        Some(Ok(())),
        AgentLaunchRoute::ClaudeMessaging,
        vec![
            Call::Wait("example/repo:@1.%2".to_string()),
            Call::Send(connection(), "prompt".to_string()),
        ]
    )]
    #[case::registry_failure(
        Err(anyhow::anyhow!("registry unavailable")),
        None,
        AgentLaunchRoute::ClaudeArgvFallback {
            reason: "registry unavailable".to_string(),
        },
        vec![Call::Wait("example/repo:@1.%2".to_string())]
    )]
    #[case::delivery_failure(
        Ok(connection()),
        Some(Err(anyhow::anyhow!("socket unavailable"))),
        AgentLaunchRoute::ClaudeArgvFallback {
            reason: "socket unavailable".to_string(),
        },
        vec![
            Call::Wait("example/repo:@1.%2".to_string()),
            Call::Send(connection(), "prompt".to_string()),
        ]
    )]
    fn messaging_routes(
        #[case] connection_result: anyhow::Result<PeerConnection>,
        #[case] delivery: Option<anyhow::Result<()>>,
        #[case] expected_route: AgentLaunchRoute,
        #[case] expected_calls: Vec<Call>,
    ) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let client = StubClient {
            connection: connection_result,
            delivery,
            calls: Arc::clone(&calls),
        };
        let launch = Launch::prepare_with(Engine::Claude, Some("prompt"), 1, || Box::new(client));

        let route = launch.finish(Some("example/repo:@1.%2"));
        let actual_calls = calls.lock().unwrap().clone();

        assert_eq!((route, actual_calls), (expected_route, expected_calls));
    }

    #[test]
    fn missing_tmux_location_falls_back_before_registry_lookup() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let client = StubClient {
            connection: Ok(connection()),
            delivery: Some(Ok(())),
            calls: Arc::clone(&calls),
        };
        let launch = Launch::prepare_with(Engine::Claude, Some("prompt"), 1, || Box::new(client));

        let route = launch.finish(None);
        let actual_calls = calls.lock().unwrap().clone();

        assert_eq!(
            (route, actual_calls),
            (
                AgentLaunchRoute::ClaudeArgvFallback {
                    reason: "Claude messaging launch has no tmux location".to_string(),
                },
                Vec::new(),
            ),
        );
    }

    #[rstest]
    fn successful_delivery_removes_prompt_file(prompt_file: (tempfile::TempDir, PathBuf)) {
        let (_dir, prompt_path) = prompt_file;
        let launch = Launch::prepare_with(Engine::Claude, Some("prompt"), 1, || {
            Box::new(StubClient {
                connection: Ok(connection()),
                delivery: Some(Ok(())),
                calls: Arc::new(Mutex::new(Vec::new())),
            })
        });

        let route = launch.finish_and_recover_with(
            RecoverySpec {
                effort: None,
                prompt_file: Some(&prompt_path),
                command: "claude",
                model: None,
                pane_id: Some("%2"),
                tmux_location: Some("example/repo:@1.%2"),
                env_vars: &[],
            },
            |_, _, _| panic!("respawn should not run"),
        );

        assert_eq!(
            (
                route.map_err(|error| error.to_string()),
                prompt_path.exists()
            ),
            (Ok(AgentLaunchRoute::ClaudeMessaging), false),
        );
    }

    #[rstest]
    fn failed_delivery_respawns_with_recoverable_prompt_file(
        prompt_file: (tempfile::TempDir, PathBuf),
    ) {
        let (_dir, prompt_path) = prompt_file;
        let launch = Launch::prepare_with(Engine::Claude, Some("prompt"), 1, || {
            Box::new(StubClient {
                connection: Ok(connection()),
                delivery: Some(Err(anyhow::anyhow!("socket unavailable"))),
                calls: Arc::new(Mutex::new(Vec::new())),
            })
        });
        let mut respawned = None;

        let (route, expected_command) =
            temp_env::with_var("SHELL", Some("/bin/example-shell"), || {
                let route = launch.finish_and_recover_with(
                    RecoverySpec {
                        effort: Some(ReasoningEffort::Max),
                        prompt_file: Some(&prompt_path),
                        command: "claude",
                        model: Some("example-model"),
                        pane_id: Some("%2"),
                        tmux_location: Some("example/repo:@1.%2"),
                        env_vars: &[("EXAMPLE_KEY", "example-value")],
                    },
                    |pane_id, command, env_vars| {
                        respawned = Some((
                            pane_id.to_string(),
                            command.to_string(),
                            env_vars
                                .iter()
                                .map(|(key, value)| (key.to_string(), value.to_string()))
                                .collect(),
                        ));
                        Ok(())
                    },
                );
                let argv_command = format!(
                    "claude --model example-model --effort max \"$(cat {})\" && rm {}",
                    prompt_path.display(),
                    prompt_path.display(),
                );
                (route, wrap_in_interactive_shell(&argv_command).unwrap())
            });

        assert_eq!(
            (
                route.map_err(|error| error.to_string()),
                prompt_path.exists(),
                respawned,
            ),
            (
                Ok(AgentLaunchRoute::ClaudeArgvFallback {
                    reason: "socket unavailable".to_string(),
                }),
                true,
                Some((
                    "%2".to_string(),
                    expected_command,
                    vec![("EXAMPLE_KEY".to_string(), "example-value".to_string())],
                )),
            ),
        );
    }
}
