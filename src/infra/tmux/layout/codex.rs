use std::path::Path;

use crate::commands::agent::codex_steer;
use crate::commands::agent::types::{Engine, ReasoningEffort};

/// How an agent process received its initial prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentLaunchRoute {
    /// No initial prompt was provided, or the engine is not Codex.
    Standard,
    /// Codex attached to the shared app-server and received its first turn by RPC.
    CodexDaemon,
    /// Codex was launched with the prompt in argv because the daemon path failed.
    CodexArgvFallback { reason: String },
}

impl AgentLaunchRoute {
    pub fn display_suffix(&self) -> String {
        match self {
            Self::Standard => String::new(),
            Self::CodexDaemon => " (Codex daemon)".to_string(),
            Self::CodexArgvFallback { reason } => {
                format!(" (Codex argv fallback: {reason})")
            }
        }
    }
}

pub(super) struct Launch {
    state: LaunchState,
}

enum LaunchState {
    Standard,
    Connected(Box<codex_steer::Client>),
    Fallback(String),
}

impl Launch {
    pub(super) fn prepare(engine: Engine, prompt: Option<&str>, pane_count: usize) -> Self {
        let state = if engine != Engine::Codex || prompt.is_none() {
            LaunchState::Standard
        } else if pane_count != 1 {
            LaunchState::Fallback(format!(
                "daemon launch requires exactly one Codex pane, found {pane_count}"
            ))
        } else {
            match codex_steer::Client::connect() {
                Ok(client) => LaunchState::Connected(Box::new(client)),
                Err(error) => LaunchState::Fallback(error.to_string()),
            }
        };
        Self { state }
    }

    pub(super) fn uses_daemon(&self) -> bool {
        matches!(self.state, LaunchState::Connected(_))
    }

    pub(super) fn finish(
        self,
        cwd: &Path,
        prompt: Option<&str>,
        effort: Option<ReasoningEffort>,
    ) -> AgentLaunchRoute {
        match self.state {
            LaunchState::Standard => AgentLaunchRoute::Standard,
            LaunchState::Fallback(reason) => AgentLaunchRoute::CodexArgvFallback { reason },
            LaunchState::Connected(mut client) => {
                let Some(prompt) = prompt else {
                    return AgentLaunchRoute::CodexArgvFallback {
                        reason: "Codex daemon launch lost its initial prompt".to_string(),
                    };
                };
                let result = (|| {
                    let thread_id = client.wait_for_thread_started(cwd)?;
                    client
                        .start_turn(&thread_id, prompt, effort)
                        .map_err(anyhow::Error::new)
                })();
                match result {
                    Ok(()) => AgentLaunchRoute::CodexDaemon,
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

    #[test]
    fn standard_route_has_no_suffix() {
        assert_eq!(AgentLaunchRoute::Standard.display_suffix(), "");
    }

    #[test]
    fn fallback_route_reports_its_reason() {
        assert_eq!(
            AgentLaunchRoute::CodexArgvFallback {
                reason: "daemon unavailable".to_string(),
            }
            .display_suffix(),
            " (Codex argv fallback: daemon unavailable)",
        );
    }
}
