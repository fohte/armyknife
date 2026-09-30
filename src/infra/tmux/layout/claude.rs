use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

use super::AgentLaunchRoute;
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
}

pub(super) struct RecoverySpec<'a> {
    pub prompt_file: Option<&'a Path>,
    pub pane_id: Option<&'a str>,
}

impl Launch {
    pub(super) fn prepare(
        engine: Engine,
        prompt: Option<&str>,
        pane_count: usize,
    ) -> anyhow::Result<Self> {
        Self::prepare_with(engine, prompt, pane_count, || Box::new(Client))
    }

    fn prepare_with(
        engine: Engine,
        prompt: Option<&str>,
        pane_count: usize,
        client: impl FnOnce() -> Box<dyn MessagingClient>,
    ) -> anyhow::Result<Self> {
        let state = if engine != Engine::Claude || prompt.is_none() {
            LaunchState::Standard
        } else if pane_count != 1 {
            bail!("Claude messaging launch requires exactly one Claude pane, found {pane_count}");
        } else {
            LaunchState::Messaging {
                client: client(),
                prompt: prompt.unwrap_or_default().to_string(),
            }
        };
        Ok(Self { state })
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
        self.finish_and_recover_with(spec, tmux::get_pane_registry_location)
    }

    fn finish_and_recover_with(
        self,
        spec: RecoverySpec<'_>,
        resolve_tmux_location: impl FnOnce(&str) -> Option<String>,
    ) -> anyhow::Result<AgentLaunchRoute> {
        let messaging_launch = self.uses_messaging();
        let tmux_location = if messaging_launch {
            spec.pane_id.and_then(resolve_tmux_location)
        } else {
            None
        };
        let route = self.finish(tmux_location.as_deref())?;
        match route {
            AgentLaunchRoute::ClaudeMessaging => {
                if let Some(path) = spec.prompt_file {
                    std::fs::remove_file(path).with_context(|| {
                        format!("Failed to remove prompt file {}", path.display())
                    })?;
                }
                Ok(AgentLaunchRoute::ClaudeMessaging)
            }
            route => Ok(route),
        }
    }

    fn finish(self, tmux_location: Option<&str>) -> anyhow::Result<AgentLaunchRoute> {
        match self.state {
            LaunchState::Standard => Ok(AgentLaunchRoute::Standard),
            LaunchState::Messaging { mut client, prompt } => {
                let tmux_location =
                    tmux_location.context("Claude messaging launch has no tmux location")?;
                let connection = client
                    .wait_for_connection(tmux_location)
                    .context("Failed to wait for Claude messaging connection")?;
                client
                    .send_message(&connection, &prompt)
                    .context("Failed to send initial prompt through Claude messaging")?;
                Ok(AgentLaunchRoute::ClaudeMessaging)
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
    fn preflight_routes(
        #[case] engine: Engine,
        #[case] prompt: Option<&str>,
        #[case] pane_count: usize,
        #[case] expected: AgentLaunchRoute,
    ) {
        let launch = Launch::prepare_with(engine, prompt, pane_count, || {
            panic!("client should not be created")
        })
        .expect("preflight should succeed");

        assert_eq!(
            launch
                .finish(Some("example/repo:@1.%2"))
                .map_err(|error| format!("{error:#}")),
            Ok(expected),
        );
    }

    #[test]
    fn multiple_panes_fail_before_creating_client() {
        let result = Launch::prepare_with(Engine::Claude, Some("prompt"), 2, || {
            panic!("messaging client should not be created")
        });

        assert_eq!(
            result.err().map(|error| error.to_string()),
            Some("Claude messaging launch requires exactly one Claude pane, found 2".to_string()),
        );
    }

    #[test]
    fn messaging_command_options_keep_effort_and_omit_prompt_file() {
        let launch = Launch::prepare_with(Engine::Claude, Some("prompt"), 1, || {
            Box::new(StubClient {
                connection: Ok(connection()),
                delivery: Some(Ok(())),
                calls: Arc::new(Mutex::new(Vec::new())),
            })
        })
        .expect("messaging client should be created");
        let prompt_path = Path::new("/tmp/example-prompt.txt");

        let options = launch.command_options(Some(ReasoningEffort::Max), Some(prompt_path));

        assert_eq!(options, (Some(ReasoningEffort::Max), None));
    }

    #[rstest]
    #[case::success(
        Ok(connection()),
        Some(Ok(())),
        Ok(AgentLaunchRoute::ClaudeMessaging),
        vec![
            Call::Wait("example/repo:@1.%2".to_string()),
            Call::Send(connection(), "prompt".to_string()),
        ]
    )]
    #[case::registry_failure(
        Err(anyhow::anyhow!("registry unavailable")),
        None,
        Err("Failed to wait for Claude messaging connection: registry unavailable".to_string()),
        vec![Call::Wait("example/repo:@1.%2".to_string())]
    )]
    #[case::delivery_failure(
        Ok(connection()),
        Some(Err(anyhow::anyhow!("socket unavailable"))),
        Err("Failed to send initial prompt through Claude messaging: socket unavailable".to_string()),
        vec![
            Call::Wait("example/repo:@1.%2".to_string()),
            Call::Send(connection(), "prompt".to_string()),
        ]
    )]
    fn messaging_routes(
        #[case] connection_result: anyhow::Result<PeerConnection>,
        #[case] delivery: Option<anyhow::Result<()>>,
        #[case] expected_route: Result<AgentLaunchRoute, String>,
        #[case] expected_calls: Vec<Call>,
    ) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let client = StubClient {
            connection: connection_result,
            delivery,
            calls: Arc::clone(&calls),
        };
        let launch = Launch::prepare_with(Engine::Claude, Some("prompt"), 1, || Box::new(client))
            .expect("messaging client should be created");

        let route = launch.finish(Some("example/repo:@1.%2"));
        let actual_calls = calls.lock().unwrap().clone();

        assert_eq!(
            (route.map_err(|error| format!("{error:#}")), actual_calls),
            (expected_route, expected_calls),
        );
    }

    #[test]
    fn missing_tmux_location_fails_before_registry_lookup() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let client = StubClient {
            connection: Ok(connection()),
            delivery: Some(Ok(())),
            calls: Arc::clone(&calls),
        };
        let launch = Launch::prepare_with(Engine::Claude, Some("prompt"), 1, || Box::new(client))
            .expect("messaging client should be created");

        let route = launch.finish(None);
        let actual_calls = calls.lock().unwrap().clone();

        assert_eq!(
            (route.map_err(|error| format!("{error:#}")), actual_calls),
            (
                Err("Claude messaging launch has no tmux location".to_string()),
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
        })
        .expect("messaging client should be created");

        let route = launch.finish_and_recover_with(
            RecoverySpec {
                prompt_file: Some(&prompt_path),
                pane_id: Some("%2"),
            },
            |_| Some("example/repo:@1.%2".to_string()),
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
    fn failed_delivery_returns_error_and_preserves_prompt_file(
        prompt_file: (tempfile::TempDir, PathBuf),
    ) {
        let (_dir, prompt_path) = prompt_file;
        let launch = Launch::prepare_with(Engine::Claude, Some("prompt"), 1, || {
            Box::new(StubClient {
                connection: Ok(connection()),
                delivery: Some(Err(anyhow::anyhow!("socket unavailable"))),
                calls: Arc::new(Mutex::new(Vec::new())),
            })
        })
        .expect("messaging client should be created");
        let route = launch.finish_and_recover_with(
            RecoverySpec {
                prompt_file: Some(&prompt_path),
                pane_id: Some("%2"),
            },
            |_| Some("example/repo:@1.%2".to_string()),
        );

        assert_eq!(
            (
                route.map_err(|error| format!("{error:#}")),
                prompt_path.exists(),
            ),
            (
                Err(
                    "Failed to send initial prompt through Claude messaging: socket unavailable"
                        .to_string()
                ),
                true,
            ),
        );
    }
}
