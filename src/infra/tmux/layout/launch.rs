use std::path::Path;

use super::AgentLaunchRoute;
use super::claude::{Launch as ClaudeLaunch, RecoverySpec as ClaudeRecoverySpec};
use super::codex::{Launch as CodexLaunch, RecoverySpec as CodexRecoverySpec};
use crate::commands::agent::types::{Engine, ReasoningEffort};

pub(super) struct Launch {
    state: LaunchState,
}

enum LaunchState {
    Claude(ClaudeLaunch),
    Codex(CodexLaunch),
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
        let state = match engine {
            Engine::Claude => {
                LaunchState::Claude(ClaudeLaunch::prepare(engine, prompt, pane_count))
            }
            Engine::Codex => {
                LaunchState::Codex(CodexLaunch::prepare(engine, prompt, pane_count, cwd))
            }
        };
        Self { state }
    }

    pub(super) fn uses_remote(&self) -> bool {
        match &self.state {
            LaunchState::Claude(launch) => launch.uses_messaging(),
            LaunchState::Codex(launch) => launch.uses_daemon(),
        }
    }

    pub(super) fn command_options<'a>(
        &self,
        effort: Option<ReasoningEffort>,
        prompt_file: Option<&'a Path>,
    ) -> (Option<ReasoningEffort>, Option<&'a Path>) {
        match &self.state {
            LaunchState::Claude(launch) => launch.command_options(effort, prompt_file),
            LaunchState::Codex(launch) => launch.command_options(effort, prompt_file),
        }
    }

    pub(super) fn finish_and_recover(
        self,
        spec: RecoverySpec<'_>,
    ) -> anyhow::Result<AgentLaunchRoute> {
        match self.state {
            LaunchState::Claude(launch) => launch.finish_and_recover(ClaudeRecoverySpec {
                effort: spec.effort,
                prompt_file: spec.prompt_file,
                command: spec.command,
                model: spec.model,
                pane_id: spec.pane_id,
                env_vars: spec.env_vars,
            }),
            LaunchState::Codex(launch) => launch.finish_and_recover(CodexRecoverySpec {
                cwd: spec.cwd,
                effort: spec.effort,
                prompt_file: spec.prompt_file,
                command: spec.command,
                model: spec.model,
                pane_id: spec.pane_id,
                env_vars: spec.env_vars,
            }),
        }
    }
}
