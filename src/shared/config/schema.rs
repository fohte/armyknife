use std::collections::HashMap;

use serde::Serialize;

use super::{AgentConfig, EditorConfig, NotificationConfig, OrgConfig, RepoConfig};

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

/// Generate JSON Schema for the current config keys.
pub fn generate_schema() -> schemars::Schema {
    schemars::schema_for!(ConfigSchema)
}
