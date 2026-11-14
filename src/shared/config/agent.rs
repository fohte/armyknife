use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::commands::agent::types::Engine;

use super::{
    AgentLaunchProfileConfig, AgentProfileTypeConfig, AgentStatusConfig, AgentWorkTypeConfig,
    AgentWorkTypeMarkerConfig, AutoCompactConfig, AutoPauseConfig, CodexConfig, WorktreeConfig,
};

/// Configuration for reminders sent when a background task exceeds its timeout.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BgTaskReminderConfig {
    /// How long a background task may run before a reminder is sent.
    /// Accepts human-friendly durations such as "13m". When omitted, reminders are disabled.
    #[serde(default)]
    pub timeout: Option<String>,

    /// Message sent when a background task exceeds `timeout`.
    #[serde(default = "default_bg_task_reminder_message")]
    #[cfg_attr(
        feature = "schema-gen",
        schemars(default = "default_bg_task_reminder_message")
    )]
    pub message: String,

    /// Command fragments that skip reminders when they appear in the command line.
    #[serde(default = "default_bg_task_reminder_excluded_commands")]
    #[cfg_attr(
        feature = "schema-gen",
        schemars(default = "default_bg_task_reminder_excluded_commands")
    )]
    pub excluded_commands: Vec<String>,
}

impl Default for BgTaskReminderConfig {
    fn default() -> Self {
        Self {
            timeout: None,
            message: default_bg_task_reminder_message(),
            excluded_commands: default_bg_task_reminder_excluded_commands(),
        }
    }
}

fn default_bg_task_reminder_message() -> String {
    "This is a reminder, not an error: this background task has been running for a long time. Check its output and the state of what it is monitoring. Stop it if it is stuck.".to_string()
}

fn default_bg_task_reminder_excluded_commands() -> Vec<String> {
    Vec::new()
}

/// `a agent` settings.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    /// Fallback CLI when `--type` and `--engine` are omitted and the matching
    /// launch profile's engine is unset (default: "claude"). Overridable for a
    /// single invocation via `ARMYKNIFE_AGENT__DEFAULT_ENGINE`.
    #[serde(default)]
    pub default_engine: Engine,

    /// Profile selected by `a agent profile use` for launches with `--type`.
    #[serde(default)]
    pub active_profile: Option<String>,

    /// Named profiles containing execution settings for each arbitrary type.
    #[serde(default)]
    pub profiles: HashMap<String, HashMap<String, AgentProfileTypeConfig>>,

    /// Message template sent to the delegator when a delegated branch's PR merges.
    /// Supports `{branch}` and `{pr_url}` placeholders. When unset, no notification is sent.
    #[serde(default)]
    pub merge_notification: Option<String>,

    /// Instruction returned by the Stop hook when it blocks a session.
    /// At least one status must be configured.
    #[serde(default)]
    pub stop_hook_instruction: Option<String>,

    /// Reminder settings for background tasks that run longer than expected.
    #[serde(default)]
    pub bg_task_reminder: BgTaskReminderConfig,

    /// Defaults applied to `a agent new --engine codex` sessions only.
    #[serde(default)]
    pub codex: CodexConfig,

    /// Launch defaults keyed by the `--kind` value passed to `a agent new`.
    #[serde(default)]
    pub launch_profiles: HashMap<String, AgentLaunchProfileConfig>,

    /// Display configuration for agent work types, keyed by skill name.
    #[serde(default)]
    pub work_types: HashMap<String, AgentWorkTypeConfig>,

    /// Display configuration used in `a agent watch` when a session has no recorded work type.
    #[serde(default)]
    pub work_type_fallback: Option<AgentWorkTypeMarkerConfig>,

    /// Marker shown after the name of a session running in vibe mode.
    #[serde(default)]
    pub vibe_marker: Option<AgentWorkTypeMarkerConfig>,

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
    pub(super) fn selected_profile_value(&self, path: Option<&str>) -> Option<serde_json::Value> {
        let active_profile = self.active_profile.as_deref()?;
        let profile = self.profiles.get(active_profile)?;
        let value = serde_json::to_value(profile).ok()?;

        match path {
            Some(path) => super::resolve_json_path(&value, path),
            None => Some(value),
        }
    }

    /// Look up the display configuration for a skill name.
    pub fn work_type(&self, skill_name: &str) -> Option<&AgentWorkTypeConfig> {
        self.work_types.get(skill_name)
    }
}
