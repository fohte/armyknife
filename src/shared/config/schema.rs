use std::collections::HashMap;

use serde::Serialize;

use super::{
    AgentConfig, AutoCompactConfig, AutoPauseConfig, EditorConfig, LayoutNode, NotificationConfig,
    OrgConfig, RepoConfig, default_branch_prefix, default_worktrees_dir,
};

#[derive(Default, Serialize, schemars::JsonSchema)]
#[schemars(
    rename = "Config",
    description = "Top-level configuration for armyknife."
)]
#[serde(deny_unknown_fields)]
struct ConfigSchema {
    /// `a agent` settings.
    #[serde(default)]
    agent: AgentConfig,
    /// Legacy worktree settings. Use `agent.worktree` instead.
    #[serde(default)]
    wm: LegacyWmSchema,
    /// Legacy session settings. Use `agent.auto_pause` and `agent.auto_compact` instead.
    #[serde(default)]
    cc: LegacyCcSchema,
    /// Terminal/editor settings for human-in-the-loop reviews.
    #[serde(default)]
    editor: EditorConfig,
    /// Notification settings.
    #[serde(default)]
    notification: NotificationConfig,
    /// Per-repository configuration, keyed by "owner/repo".
    #[serde(default)]
    repos: HashMap<String, RepoConfig>,
    /// Per-organization configuration, keyed by GitHub owner (org or user).
    #[serde(default)]
    orgs: HashMap<String, OrgConfig>,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LegacyWmSchema {
    #[serde(default = "default_worktrees_dir")]
    #[schemars(default = "default_worktrees_dir")]
    worktrees_dir: String,
    #[serde(default = "default_branch_prefix")]
    #[schemars(default = "default_branch_prefix")]
    branch_prefix: String,
    #[serde(default)]
    layout: LayoutNode,
    #[serde(default)]
    repos_root: Option<String>,
}

impl Default for LegacyWmSchema {
    fn default() -> Self {
        Self {
            worktrees_dir: default_worktrees_dir(),
            branch_prefix: default_branch_prefix(),
            layout: LayoutNode::default(),
            repos_root: None,
        }
    }
}

#[derive(Default, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LegacyCcSchema {
    #[serde(default)]
    auto_pause: AutoPauseConfig,
    #[serde(default)]
    auto_compact: AutoCompactConfig,
}

/// Generate JSON Schema for the current config keys and their legacy aliases.
pub fn generate_schema() -> schemars::Schema {
    schemars::schema_for!(ConfigSchema)
}
