use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::commands::agent::types::Engine;

use super::{
    AgentStatusConfig, AgentWorkTypeConfig, AutoCompactConfig, AutoPauseConfig, CodexConfig,
    WorktreeConfig,
};

/// `a agent` settings.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    /// Coding agent CLI `a agent new` launches when `--engine` is omitted
    /// (default: "claude"). Overridable for a single invocation via
    /// `ARMYKNIFE_AGENT__DEFAULT_ENGINE`.
    #[serde(default)]
    pub default_engine: Engine,

    /// Message template sent to the delegator when a delegated branch's PR merges.
    /// Supports `{branch}` and `{pr_url}` placeholders. When unset, no notification is sent.
    #[serde(default)]
    pub merge_notification: Option<String>,

    /// Instruction returned by the Stop hook when it blocks a session.
    /// The hook blocks only when this is set and at least one status is configured.
    #[serde(default)]
    pub stop_hook_instruction: Option<String>,

    /// Defaults applied to `a agent new --engine codex` sessions only.
    #[serde(default)]
    pub codex: CodexConfig,

    /// Display configuration for agent work types, keyed by skill name.
    #[serde(default)]
    pub work_types: HashMap<String, AgentWorkTypeConfig>,

    /// Display configuration used in `a agent watch` when a session has no recorded work type.
    #[serde(default)]
    pub work_type_fallback: Option<AgentWorkTypeConfig>,

    /// Marker shown after the name of a session running in vibe mode.
    #[serde(default)]
    pub vibe_marker: Option<AgentWorkTypeConfig>,

    /// User-defined status names and the state each name represents.
    #[serde(default)]
    pub statuses: HashMap<String, AgentStatusConfig>,

    /// Worktree settings used by `a agent` and worktree cleanup commands.
    #[serde(default)]
    pub worktree: WorktreeConfig,

    /// Automatic pause settings for long-stopped sessions.
    #[serde(default)]
    pub auto_pause: AutoPauseConfig,

    /// Automatic `/compact` settings for idle sessions.
    #[serde(default)]
    pub auto_compact: AutoCompactConfig,
}

impl AgentConfig {
    /// Look up the display configuration for a skill name.
    pub fn work_type(&self, skill_name: &str) -> Option<&AgentWorkTypeConfig> {
        self.work_types.get(skill_name)
    }

    /// Look up the work type displayed in `a agent watch`, including its fallback.
    pub fn watch_work_type(&self, skill_name: Option<&str>) -> Option<&AgentWorkTypeConfig> {
        match skill_name {
            Some(skill_name) => self.work_type(skill_name),
            None => self.work_type_fallback.as_ref(),
        }
    }
}
