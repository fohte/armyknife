use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::commands::agent::types::Engine;
use crate::commands::ai::review::reviewer::Reviewer;

mod agent;
mod agent_status;
mod codex;
mod editor_focus;
mod env_overlay;
mod launch_profile;
#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::HashMap;

    use rstest::fixture;

    use super::{AgentConfig, AgentStatusConfig, AgentStatusState};

    #[fixture]
    pub(crate) fn agent_config() -> AgentConfig {
        AgentConfig {
            statuses: HashMap::from([
                (
                    "needs_attention".to_string(),
                    AgentStatusConfig {
                        state: AgentStatusState::User,
                    },
                ),
                (
                    "awaiting_build".to_string(),
                    AgentStatusConfig {
                        state: AgentStatusState::Waiting,
                    },
                ),
                (
                    "finished_work".to_string(),
                    AgentStatusConfig {
                        state: AgentStatusState::Done,
                    },
                ),
            ]),
            ..AgentConfig::default()
        }
    }
}

#[cfg(feature = "schema-gen")]
mod schema;
mod work_type;
pub use agent::{
    AgentConfig, BgTaskReminderConfig, CodexStatusInferenceConfig, StatusInferenceStatusConfig,
};
pub use agent_status::{AgentStatusConfig, AgentStatusState};
pub use codex::CodexConfig;
use env_overlay::env_overlay;
pub use launch_profile::{
    AgentLaunchEngineConfig, AgentLaunchProfileConfig, AgentProfileTypeConfig,
    AgentProfileTypeEngine, ResolvedAgentProfileType, ResolvedLaunchOptions,
};
#[cfg(feature = "schema-gen")]
pub use schema::generate_schema;
pub use work_type::{AgentWorkTypeColor, AgentWorkTypeConfig, AgentWorkTypeNamedColor};

pub(crate) fn format_sorted_names<'a>(names: impl Iterator<Item = &'a String>) -> String {
    let mut names = names.map(String::as_str).collect::<Vec<_>>();
    names.sort_unstable();
    if names.is_empty() {
        "(none)".to_string()
    } else {
        names.join(", ")
    }
}

/// Top-level configuration for armyknife.
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// `a agent` settings.
    #[serde(default)]
    pub agent: AgentConfig,

    /// Terminal/editor settings for human-in-the-loop reviews.
    #[serde(default)]
    pub editor: EditorConfig,

    /// Notification settings.
    #[serde(default)]
    pub notification: NotificationConfig,

    /// Per-repository configuration, keyed by "owner/repo".
    #[serde(default)]
    pub repos: HashMap<String, RepoConfig>,

    /// Per-organization configuration, keyed by GitHub owner (org or user).
    #[serde(default)]
    pub orgs: HashMap<String, OrgConfig>,
}

impl Config {
    /// Look up a config value by dot-separated key path.
    /// For `repo.*` keys, `repo_id` (e.g. "owner/repo") selects which repo entry to use.
    /// For `org.*` keys, `repo_id` 's owner segment selects the org entry.
    /// `agent.profile` keys resolve against the selected agent profile.
    /// Returns the raw JSON value at the path, or None if the path is missing
    /// or resolves to null. Callers format scalars and collections as needed.
    pub fn get_value(&self, key: &str, repo_id: Option<&str>) -> Option<serde_json::Value> {
        if key == "agent.profile" {
            return self.agent.selected_profile_value(None);
        }

        if let Some(profile_path) = key.strip_prefix("agent.profile.") {
            return self.agent.selected_profile_value(Some(profile_path));
        }

        if key == "repo.vibe" {
            let (owner, repo) = repo_id?.split_once('/')?;
            return Some(serde_json::Value::Bool(
                self.is_vibe_allowed_for_repo(owner, repo),
            ));
        }

        if let Some(repo_key) = key.strip_prefix("repo.") {
            // Fall back to RepoConfig::default() when no entry exists for this
            // repo, so per-field serde defaults (e.g. direct_commit = false)
            // are honored even for repos that have no `repos.<owner>/<repo>`
            // section in the config file.
            repo_id?;
            let default_repo_config = RepoConfig::default();
            let repo_config = repo_id
                .and_then(|id| self.repos.get(id))
                .unwrap_or(&default_repo_config);
            let value = serde_json::to_value(repo_config).ok()?;
            resolve_json_path(&value, repo_key)
        } else if let Some(org_key) = key.strip_prefix("org.") {
            let owner = repo_id.and_then(|id| id.split_once('/').map(|(o, _)| o))?;
            let default_org_config = OrgConfig::default();
            let org_config = self.orgs.get(owner).unwrap_or(&default_org_config);
            let value = serde_json::to_value(org_config).ok()?;
            resolve_json_path(&value, org_key)
        } else {
            let value = serde_json::to_value(self).ok()?;
            resolve_json_path(&value, key)
        }
    }

    /// Resolve whether vibe mode is allowed for a repository.
    /// GitHub owner and repository names are compared without case sensitivity for this policy.
    /// Other config lookups retain their existing key matching behavior.
    /// An explicit repository setting overrides its owner's setting.
    pub fn is_vibe_allowed_for_repo(&self, owner: &str, repo: &str) -> bool {
        let repo_id = format!("{owner}/{repo}");
        let org_vibe =
            strictest_vibe(self.orgs.iter(), owner, |config| Some(config.vibe)).unwrap_or(false);

        strictest_vibe(self.repos.iter(), &repo_id, |config| config.vibe).unwrap_or(org_vibe)
    }

    /// Resolve the list of reviewers to wait for, given a repo identifier (`owner/repo`).
    /// Resolution order: repo (`repos.<owner>/<repo>.ai.review.reviewers`) -> org
    /// (`orgs.<owner>.ai.review.reviewers`) -> None (caller falls back to its own default).
    pub fn resolve_reviewers(&self, owner: &str, repo: &str) -> Option<Vec<Reviewer>> {
        let repo_id = format!("{owner}/{repo}");
        if let Some(repo_cfg) = self.repos.get(&repo_id)
            && let Some(reviewers) = repo_cfg.ai.review.reviewers.as_ref()
        {
            return Some(reviewers.clone());
        }
        if let Some(org_cfg) = self.orgs.get(owner)
            && let Some(reviewers) = org_cfg.ai.review.reviewers.as_ref()
        {
            return Some(reviewers.clone());
        }
        None
    }
}

fn strictest_vibe<'a, ConfigType: 'a>(
    entries: impl Iterator<Item = (&'a String, &'a ConfigType)>,
    key: &str,
    vibe: impl Fn(&ConfigType) -> Option<bool>,
) -> Option<bool> {
    entries
        .filter(|(configured_key, _)| configured_key.eq_ignore_ascii_case(key))
        .filter_map(|(_, config)| vibe(config))
        .reduce(|allowed, next| allowed && next)
}

/// Resolve a dot-separated path against a JSON value. Null is treated as
/// missing so callers (and the `repo.language` fallback in `run_get`) can
/// distinguish "set to a value" from "absent / explicitly null".
fn resolve_json_path(value: &serde_json::Value, path: &str) -> Option<serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    if current.is_null() {
        None
    } else {
        Some(current.clone())
    }
}

/// Worktree configuration.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorktreeConfig {
    /// Worktree directory name (default: ".worktrees").
    #[serde(default = "default_worktrees_dir")]
    #[cfg_attr(feature = "schema-gen", schemars(default = "default_worktrees_dir"))]
    pub dir: String,

    /// Branch prefix (default: "fohte/").
    #[serde(default = "default_branch_prefix")]
    #[cfg_attr(feature = "schema-gen", schemars(default = "default_branch_prefix"))]
    pub branch_prefix: String,

    /// tmux pane layout definition.
    #[serde(default)]
    pub layout: LayoutNode,

    /// Root directory containing git repositories.
    /// Used by `a agent clean --all` to discover repositories.
    /// Falls back to GHQ_ROOT env, git config ghq.root, or ~/ghq.
    #[serde(default)]
    pub repos_root: Option<String>,
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            dir: default_worktrees_dir(),
            branch_prefix: default_branch_prefix(),
            layout: LayoutNode::default(),
            repos_root: None,
        }
    }
}

/// Layout tree node: either a single pane (leaf) or a split (internal node).
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum LayoutNode {
    /// Split node with two children.
    Split(SplitConfig),
    /// Leaf node: a single pane.
    Pane(PaneConfig),
}

impl Default for LayoutNode {
    fn default() -> Self {
        LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "nvim".to_string(),
                focus: true,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude".to_string(),
                focus: false,
            })),
        })
    }
}

/// Split configuration with direction and two child nodes.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SplitConfig {
    /// Split direction ("horizontal" or "vertical").
    pub direction: SplitDirection,
    /// First child (left for horizontal, top for vertical).
    pub first: Box<LayoutNode>,
    /// Second child (right for horizontal, bottom for vertical).
    pub second: Box<LayoutNode>,
}

/// Split direction.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    /// Horizontal split (tmux split-window -h).
    Horizontal,
    /// Vertical split (tmux split-window -v).
    Vertical,
}

/// Configuration for a single tmux pane.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PaneConfig {
    /// Command to run in the pane.
    pub command: String,
    /// Whether to focus this pane (default: false).
    #[serde(default)]
    pub focus: bool,
}

/// Terminal emulator to use for human-in-the-loop reviews.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Terminal {
    /// WezTerm terminal emulator.
    #[default]
    Wezterm,
    /// Ghostty terminal emulator.
    Ghostty,
}

impl Terminal {
    /// Returns the default macOS app name for focus-on-click.
    pub fn default_focus_app(&self) -> &str {
        match self {
            Terminal::Wezterm => "WezTerm",
            Terminal::Ghostty => "Ghostty",
        }
    }
}

/// Terminal/editor configuration for human-in-the-loop reviews.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EditorConfig {
    /// Terminal emulator (default: "wezterm").
    #[serde(default)]
    pub terminal: Terminal,

    /// Editor command (default: "nvim").
    #[serde(default = "default_editor_command")]
    #[cfg_attr(feature = "schema-gen", schemars(default = "default_editor_command"))]
    pub editor_command: String,

    /// App name to focus on notification click (macOS only).
    /// If omitted, derived from the terminal setting.
    #[serde(default)]
    pub focus_app: Option<String>,
}

impl EditorConfig {
    /// Returns the app name to focus on notification click.
    /// Uses the explicit setting if present, otherwise derives from the terminal.
    pub fn focus_app(&self) -> &str {
        self.focus_app
            .as_deref()
            .unwrap_or_else(|| self.terminal.default_focus_app())
    }
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            terminal: Terminal::default(),
            editor_command: default_editor_command(),
            focus_app: None,
        }
    }
}

/// Notification configuration.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NotificationConfig {
    /// Whether notifications are enabled (default: true).
    #[serde(default = "default_true")]
    #[cfg_attr(feature = "schema-gen", schemars(default = "default_true"))]
    pub enabled: bool,

    /// Notification sound name (default: "Glass"). Empty string for silent.
    #[serde(default = "default_notification_sound")]
    #[cfg_attr(
        feature = "schema-gen",
        schemars(default = "default_notification_sound")
    )]
    pub sound: String,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            sound: default_notification_sound(),
        }
    }
}

/// Per-repository configuration.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    /// Language for commit messages and PR content (e.g., "ja", "en").
    #[serde(default)]
    pub language: Option<String>,

    /// Whether direct commits to the default branch (e.g., master/main) are allowed.
    /// Consumed by external git hooks; armyknife only stores and exposes the value.
    #[serde(default)]
    #[cfg_attr(feature = "schema-gen", schemars(default))]
    pub direct_commit: bool,

    /// Whether this fork is maintained independently without contributing upstream.
    /// Consumed by external tools; armyknife only stores and exposes the value.
    #[serde(default)]
    #[cfg_attr(feature = "schema-gen", schemars(default))]
    pub standalone_fork: bool,

    /// Whether vibe mode is allowed for this repository. When omitted, the org setting applies.
    #[serde(default)]
    pub vibe: Option<bool>,

    /// AI-related per-repo overrides (e.g., reviewer set for `a ai review wait`).
    #[serde(default)]
    pub ai: AiConfig,
}

/// Per-organization (GitHub owner) configuration.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OrgConfig {
    /// Whether vibe mode is allowed for repositories owned by this organization (default: false).
    #[serde(default)]
    #[cfg_attr(feature = "schema-gen", schemars(default))]
    pub vibe: bool,

    /// AI-related per-org defaults (e.g., reviewer set for `a ai review wait`).
    #[serde(default)]
    pub ai: AiConfig,
}

/// Settings under the `ai` key (used in both org and repo scopes).
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiConfig {
    /// Settings for `a ai review` commands.
    #[serde(default)]
    pub review: AiReviewConfig,
}

/// Settings for `a ai review` commands.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiReviewConfig {
    /// Default reviewers for `a ai review wait` and `a ai review request`.
    /// `None` means "fall back to the next layer" (org -> built-in default).
    #[serde(default)]
    pub reviewers: Option<Vec<Reviewer>>,
}

/// Configuration for automatically pausing sessions that stay in the Stopped
/// state for longer than `timeout`.
///
/// A periodic `a agent sweep` run (typically driven by launchd) scans all sessions,
/// requests shutdown of any agent process whose session has been Stopped for
/// longer than `timeout`, and flips the session status to Paused so that
/// `a agent resume` can restore it later.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AutoPauseConfig {
    /// Whether automatic pausing is enabled (default: true).
    #[serde(default = "default_true")]
    #[cfg_attr(feature = "schema-gen", schemars(default = "default_true"))]
    pub enabled: bool,

    /// How long a session must stay in Stopped before being paused.
    /// Accepts human-friendly durations parsed by the `humantime` style parser
    /// built into armyknife, e.g., "30s", "10m", "1h30m".
    /// Default: "30m".
    #[serde(default = "default_auto_pause_timeout")]
    #[cfg_attr(
        feature = "schema-gen",
        schemars(default = "default_auto_pause_timeout")
    )]
    pub timeout: String,
}

impl Default for AutoPauseConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            timeout: default_auto_pause_timeout(),
        }
    }
}

fn default_auto_pause_timeout() -> String {
    "30m".to_string()
}

/// Configuration for automatically running `/compact` against sessions that
/// have been idle for `idle_timeout` while the prompt cache is still warm.
///
/// The Stop hook spawns a detached `a agent auto-compact schedule` process for
/// each Stop event. After `idle_timeout` of inactivity (anchored on the Stop
/// hook fire time), it SIGTERMs the live `claude` process and then re-runs
/// `claude -r <id> -p "/compact"` so that the compaction itself benefits from
/// the still-warm prompt cache.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AutoCompactConfig {
    /// Whether automatic compaction is enabled (default: true).
    #[serde(default = "default_true")]
    #[cfg_attr(feature = "schema-gen", schemars(default = "default_true"))]
    pub enabled: bool,

    /// How long a session must stay idle (since the last Stop hook) before
    /// auto-compact fires. Should be slightly less than the prompt cache TTL
    /// so the resulting `/compact` invocation hits a warm cache (Claude Code
    /// subscriptions: 5m TTL → default 4m30s).
    #[serde(default = "default_auto_compact_idle_timeout")]
    #[cfg_attr(
        feature = "schema-gen",
        schemars(default = "default_auto_compact_idle_timeout")
    )]
    pub idle_timeout: String,

    /// Minimum context size (in tokens) required before auto-compact fires.
    /// Sessions whose latest assistant turn shows a smaller prompt than this
    /// are skipped — compacting a tiny context wastes a turn and discards
    /// useful state for no benefit. Measured against
    /// `input + cache_read + cache_creation + output` of the most recent
    /// assistant entry in the transcript, so it tracks the actual prompt the
    /// next turn would send regardless of which model context window is in
    /// use (200k vs 1M).
    /// Default: 180000.
    #[serde(default = "default_auto_compact_min_context_tokens")]
    #[cfg_attr(
        feature = "schema-gen",
        schemars(default = "default_auto_compact_min_context_tokens")
    )]
    pub min_context_tokens: u64,
}

impl Default for AutoCompactConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            idle_timeout: default_auto_compact_idle_timeout(),
            min_context_tokens: default_auto_compact_min_context_tokens(),
        }
    }
}

fn default_auto_compact_idle_timeout() -> String {
    "4m30s".to_string()
}

fn default_auto_compact_min_context_tokens() -> u64 {
    180_000
}

fn default_worktrees_dir() -> String {
    ".worktrees".to_string()
}

fn default_branch_prefix() -> String {
    "fohte/".to_string()
}

fn default_true() -> bool {
    true
}

fn default_notification_sound() -> String {
    "Glass".to_string()
}

fn default_editor_command() -> String {
    "nvim".to_string()
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Failed to read config file (permission error, etc.)
    #[error("Failed to read config file {path}: {source}")]
    ReadError {
        path: PathBuf,
        source: std::io::Error,
    },

    /// YAML parse error
    #[error("Invalid config file {path}: {message}")]
    ParseError { path: PathBuf, message: String },

    /// Deserialization error caused by an `ARMYKNIFE_*` environment variable
    /// overlay, kept distinct from `ParseError` so the message doesn't blame a
    /// YAML file for a value that actually came from the environment.
    #[error("Invalid configuration from ARMYKNIFE_* environment variables: {message}")]
    EnvParseError { message: String },
}

/// Load configuration from ~/.config/armyknife/, then apply the
/// `ARMYKNIFE_*` environment variable overlay (see `env_overlay`) on top.
/// Returns Config::default() if there is no config directory, no YAML files,
/// and no relevant environment variables.
pub fn load_config() -> anyhow::Result<Config> {
    load_config_excluding_file(None)
}

/// Prevent a local overlay from making an otherwise unknown launch kind appear configured.
pub(crate) fn load_config_without_file(file_name: &str) -> anyhow::Result<Config> {
    load_config_excluding_file(Some(file_name))
}

fn load_config_excluding_file(excluded_file: Option<&str>) -> anyhow::Result<Config> {
    let dir = super::dirs::config_dir().map(|d| d.join("armyknife"));
    let yaml_merged = match &dir {
        Some(dir) => merged_yaml_from_dir_excluding(dir, excluded_file)?,
        None => None,
    };
    // Unreachable when `dir` is None: `yaml_merged` is also None in that case,
    // and `deserialize_config(None, _)` never looks at `blame_path`.
    let blame_path = dir.unwrap_or_default();

    let config = deserialize_config_with_overlay(yaml_merged, &blame_path, env_overlay())?;
    config
        .agent
        .validate()
        .map_err(|message| anyhow::anyhow!("Invalid agent configuration: {message}"))?;
    Ok(config)
}

/// Load configuration and warn before falling back to defaults on failure.
pub(crate) fn load_config_or_default() -> Config {
    load_config().unwrap_or_else(|error| {
        tracing::warn!(%error, "Failed to load configuration; using defaults");
        Config::default()
    })
}

fn deserialize_config_with_overlay(
    yaml_merged: Option<serde_yaml::Value>,
    blame_path: &Path,
    overlay: Option<serde_yaml::Value>,
) -> anyhow::Result<Config> {
    let Some(overlay) = overlay else {
        return deserialize_config(yaml_merged, blame_path);
    };
    let combined = match yaml_merged.clone() {
        Some(base) => merge_yaml(base, overlay),
        None => overlay,
    };
    match serde_yaml::from_value::<Config>(combined) {
        Ok(config) => Ok(config),
        // The env overlay merges cleanly into the YAML document, so a single
        // deserialize error can't tell which side caused it. Re-deserialize
        // the YAML-only value to find out: if that succeeds, the environment
        // is at fault.
        Err(error) => {
            if deserialize_config(yaml_merged, blame_path).is_ok() {
                Err(ConfigError::EnvParseError {
                    message: error.to_string(),
                }
                .into())
            } else {
                Err(ConfigError::ParseError {
                    path: blame_path.to_path_buf(),
                    message: error.to_string(),
                }
                .into())
            }
        }
    }
}

/// Load configuration from a specific directory.
///
/// Reads every `*.yaml` / `*.yml` file directly under `dir` (subdirectories are
/// ignored), sorts them by file name (case-sensitive), and deep-merges them in
/// order so that later files override earlier ones. The merged value is then
/// deserialized into `Config`, applying `deny_unknown_fields` over the merged
/// document.
///
/// Returns `Config::default()` if `dir` does not exist or contains no matching
/// files.
///
/// Does not apply the `ARMYKNIFE_*` environment variable overlay (see
/// `load_config`) — this is also called directly from tests, which must not
/// be sensitive to the process environment.
pub fn load_config_from_dir(dir: &Path) -> anyhow::Result<Config> {
    let merged = merged_yaml_from_dir(dir)?;
    deserialize_config(merged, dir)
}

/// Read and deep-merge every `*.yaml` / `*.yml` file directly under `dir`, in
/// file name order. Returns `None` if `dir` does not exist or contains no
/// matching (non-null) files.
fn merged_yaml_from_dir(dir: &Path) -> anyhow::Result<Option<serde_yaml::Value>> {
    merged_yaml_from_dir_excluding(dir, None)
}

fn merged_yaml_from_dir_excluding(
    dir: &Path,
    excluded_file: Option<&str>,
) -> anyhow::Result<Option<serde_yaml::Value>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(ConfigError::ReadError {
                path: dir.to_path_buf(),
                source: e,
            }
            .into());
        }
    };

    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| ConfigError::ReadError {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if excluded_file.is_some_and(|name| {
            path.file_name().and_then(|filename| filename.to_str()) == Some(name)
        }) {
            continue;
        }
        // Filter by extension first to skip syscalls and permission errors on
        // unrelated files (README.md, .DS_Store, hooks/ subdirectory, etc.).
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if extension != "yaml" && extension != "yml" {
            continue;
        }
        // metadata follows symlinks so a `*.yaml` symlink to a private repo
        // file is honored, while symlinks to directories are still skipped.
        let metadata = std::fs::metadata(&path).map_err(|e| ConfigError::ReadError {
            path: path.clone(),
            source: e,
        })?;
        if !metadata.is_file() {
            continue;
        }
        paths.push(path);
    }
    paths.sort();

    let mut merged: Option<serde_yaml::Value> = None;
    for path in &paths {
        let content = std::fs::read_to_string(path).map_err(|e| ConfigError::ReadError {
            path: path.clone(),
            source: e,
        })?;
        // Empty or comment-only YAML deserializes as null; skip it so it doesn't
        // wipe out values from earlier files.
        let value: serde_yaml::Value =
            serde_yaml::from_str(&content).map_err(|e| ConfigError::ParseError {
                path: path.clone(),
                message: e.to_string(),
            })?;
        if value.is_null() {
            continue;
        }
        merged = Some(match merged {
            None => value,
            Some(base) => merge_yaml(base, value),
        });
    }

    Ok(merged)
}

/// Deserialize a merged YAML value into `Config`, or `Config::default()` if
/// `merged` is `None`. `blame_path` is attributed in any resulting
/// `ParseError` (the merged document has no single source file of its own).
fn deserialize_config(
    merged: Option<serde_yaml::Value>,
    blame_path: &Path,
) -> anyhow::Result<Config> {
    let Some(value) = merged else {
        return Ok(Config::default());
    };

    serde_yaml::from_value::<Config>(value)
        .map_err(|e| ConfigError::ParseError {
            path: blame_path.to_path_buf(),
            message: e.to_string(),
        })
        .map_err(Into::into)
}

/// Recursively merge two YAML values. Mappings merge key-by-key; any other type
/// (sequence, scalar, null) is replaced wholesale by `overlay`. This means
/// `reviewers: [devin]` cleanly overrides a previous `reviewers: [devin, coderabbit]`
/// rather than appending.
fn merge_yaml(base: serde_yaml::Value, overlay: serde_yaml::Value) -> serde_yaml::Value {
    match (base, overlay) {
        (serde_yaml::Value::Mapping(mut base_map), serde_yaml::Value::Mapping(overlay_map)) => {
            for (k, v) in overlay_map {
                let merged = match base_map.remove(&k) {
                    Some(existing) => merge_yaml(existing, v),
                    None => v,
                };
                base_map.insert(k, merged);
            }
            serde_yaml::Value::Mapping(base_map)
        }
        (_, overlay) => overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::env_overlay::env_overlay_from;
    use super::*;
    use crate::commands::agent::types::ReasoningEffort;
    use indoc::{formatdoc, indoc};
    use rstest::{fixture, rstest};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn get_value_treats_null_as_missing() {
        assert_eq!(
            Config::default().get_value("agent.active_profile", None),
            None,
        );
    }

    fn config_with_env_vars(dir: &Path, vars: &[(&str, &str)]) -> anyhow::Result<Config> {
        let yaml_merged = merged_yaml_from_dir(dir)?;
        let overlay = env_overlay_from(
            vars.iter()
                .map(|(name, value)| (name.to_string(), value.to_string())),
        );
        deserialize_config_with_overlay(yaml_merged, dir, overlay)
    }

    #[fixture]
    fn profile_config_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.yaml"),
            indoc! {"
                agent:
                  profiles:
                    selected-profile:
                      review-type:
                        engine: codex
                        model: model-ember
                        reasoning_effort: high
            "},
        )
        .unwrap();
        dir
    }

    #[test]
    fn config_default_has_expected_values() {
        let config = Config::default();

        assert_eq!(
            config,
            Config {
                agent: AgentConfig {
                    default_engine: Engine::Claude,
                    active_profile: None,
                    profiles: HashMap::new(),
                    merge_notification: None,
                    stop_hook_instruction: None,
                    status_inference_prompt: None,
                    codex_status_inference: CodexStatusInferenceConfig::default(),
                    bg_task_reminder: BgTaskReminderConfig::default(),
                    codex: CodexConfig {
                        model: None,
                        reasoning_effort: None,
                    },
                    launch_profiles: HashMap::new(),
                    work_types: HashMap::new(),
                    work_type_fallback: None,
                    vibe_marker: None,
                    statuses: HashMap::new(),
                    worktree: WorktreeConfig {
                        dir: ".worktrees".to_string(),
                        branch_prefix: "fohte/".to_string(),
                        layout: LayoutNode::Split(SplitConfig {
                            direction: SplitDirection::Horizontal,
                            first: Box::new(LayoutNode::Pane(PaneConfig {
                                command: "nvim".to_string(),
                                focus: true,
                            })),
                            second: Box::new(LayoutNode::Pane(PaneConfig {
                                command: "claude".to_string(),
                                focus: false,
                            })),
                        }),
                        repos_root: None,
                    },
                    auto_pause: AutoPauseConfig {
                        enabled: true,
                        timeout: "30m".to_string(),
                    },
                    auto_compact: AutoCompactConfig {
                        enabled: true,
                        idle_timeout: "4m30s".to_string(),
                        min_context_tokens: 180_000,
                    },
                },
                editor: EditorConfig {
                    terminal: Terminal::Wezterm,
                    editor_command: "nvim".to_string(),
                    focus_app: None,
                },
                notification: NotificationConfig {
                    enabled: true,
                    sound: "Glass".to_string(),
                },
                repos: HashMap::new(),
                orgs: HashMap::new(),
            }
        );
    }

    #[test]
    fn auto_compact_default_is_enabled_with_cache_friendly_timeout() {
        let cfg = AutoCompactConfig::default();
        assert_eq!(
            cfg,
            AutoCompactConfig {
                enabled: true,
                idle_timeout: "4m30s".to_string(),
                min_context_tokens: 180_000,
            }
        );
    }

    #[test]
    fn parse_auto_compact_yaml() {
        let yaml = indoc! {"
        agent:
          auto_compact:
            enabled: true
            idle_timeout: 3m
            min_context_tokens: 200000
    "};
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config,
            Config {
                agent: AgentConfig {
                    auto_compact: AutoCompactConfig {
                        enabled: true,
                        idle_timeout: "3m".to_string(),
                        min_context_tokens: 200_000,
                    },
                    ..Default::default()
                },
                ..Default::default()
            }
        );
    }

    #[test]
    fn parse_agent_codex_yaml() {
        let yaml = indoc! {"
            agent:
              codex:
                model: gpt-5.6-luna
                reasoning_effort: max
        "};
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.agent,
            AgentConfig {
                codex: CodexConfig {
                    model: Some("gpt-5.6-luna".to_string()),
                    reasoning_effort: Some(ReasoningEffort::Max),
                },
                ..Default::default()
            }
        );
    }

    #[test]
    fn parse_agent_merge_notification_yaml() {
        let yaml = indoc! {"
            agent:
              merge_notification: |
                Branch {branch} merged: {pr_url}
        "};
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(
            config.agent,
            AgentConfig {
                merge_notification: Some(
                    indoc! {"
                    Branch {branch} merged: {pr_url}
                "}
                    .to_string()
                ),
                ..Default::default()
            }
        );
    }

    #[test]
    fn parse_agent_stop_hook_instruction_yaml() {
        let yaml = indoc! {"
            agent:
              stop_hook_instruction: Please summarize the result before stopping.
              status_inference_prompt: 'Infer the current status and return JSON with kind and note.'
        "};
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(
            config.agent,
            AgentConfig {
                stop_hook_instruction: Some(
                    "Please summarize the result before stopping.".to_string()
                ),
                status_inference_prompt: Some(
                    "Infer the current status and return JSON with kind and note.".to_string()
                ),
                ..Default::default()
            }
        );
    }

    #[test]
    fn status_inference_prompt_round_trips_through_yaml() {
        let config = Config {
            agent: AgentConfig {
                status_inference_prompt: Some(
                    indoc! {"
                        Infer the current status from the full conversation.
                        Return one JSON object with kind and note.
                    "}
                    .to_string(),
                ),
                ..Default::default()
            },
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&config).unwrap();
        let round_tripped: Config = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(round_tripped, config);
    }

    #[fixture]
    fn valid_codex_inference_agent_config() -> AgentConfig {
        AgentConfig {
            status_inference_prompt: Some("Classify the stopped work.".to_string()),
            codex_status_inference: CodexStatusInferenceConfig {
                enabled: true,
                shadow_mode: true,
                failure_status: Some(StatusInferenceStatusConfig {
                    kind: "needs_review".to_string(),
                    note: "status inference failure investigation".to_string(),
                }),
                background_status: Some(StatusInferenceStatusConfig {
                    kind: "waiting_on_worker".to_string(),
                    note: "background task completion".to_string(),
                }),
            },
            statuses: HashMap::from([
                (
                    "needs_review".to_string(),
                    AgentStatusConfig {
                        state: AgentStatusState::User,
                    },
                ),
                (
                    "waiting_on_worker".to_string(),
                    AgentStatusConfig {
                        state: AgentStatusState::Waiting,
                    },
                ),
                (
                    "work_wrapped".to_string(),
                    AgentStatusConfig {
                        state: AgentStatusState::Done,
                    },
                ),
            ]),
            ..AgentConfig::default()
        }
    }

    #[rstest]
    fn parse_codex_status_inference_config(valid_codex_inference_agent_config: AgentConfig) {
        let yaml = indoc! {"
            agent:
              status_inference_prompt: Classify the stopped work.
              statuses:
                needs_review: { state: user }
                waiting_on_worker: { state: waiting }
                work_wrapped: { state: done }
              codex_status_inference:
                enabled: true
                shadow_mode: true
                failure_status:
                  kind: needs_review
                  note: status inference failure investigation
                background_status:
                  kind: waiting_on_worker
                  note: background task completion
        "};
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let validation = config.agent.validate();

        assert_eq!(
            (config.agent, validation),
            (valid_codex_inference_agent_config, Ok(())),
        );
    }

    #[derive(Clone, Copy)]
    enum InvalidCodexInferenceConfig {
        MissingFailureStatus,
        MissingBackgroundStatus,
        UnknownKind,
        EmptyKind,
        MultilineNote,
        UnicodeLineSeparator,
        MissingPrompt,
    }

    #[rstest]
    #[case::missing_failure_status(
        InvalidCodexInferenceConfig::MissingFailureStatus,
        Err("agent.codex_status_inference.failure_status is required when Codex inference is enabled".to_string())
    )]
    #[case::missing_background_status(
        InvalidCodexInferenceConfig::MissingBackgroundStatus,
        Err("agent.codex_status_inference.background_status is required when Codex inference is enabled".to_string())
    )]
    #[case::unknown_kind(
        InvalidCodexInferenceConfig::UnknownKind,
        Err("agent.codex_status_inference.failure_status.kind `unknown_kind` is not configured in agent.statuses".to_string())
    )]
    #[case::empty_kind(
        InvalidCodexInferenceConfig::EmptyKind,
        Err("agent.codex_status_inference.failure_status.kind must not be empty".to_string())
    )]
    #[case::multiline_note(
        InvalidCodexInferenceConfig::MultilineNote,
        Err("agent.codex_status_inference.failure_status.note must be a single line".to_string())
    )]
    #[case::unicode_line_separator(
        InvalidCodexInferenceConfig::UnicodeLineSeparator,
        Err("agent.codex_status_inference.failure_status.note must be a single line".to_string())
    )]
    #[case::missing_prompt(
        InvalidCodexInferenceConfig::MissingPrompt,
        Err("agent.status_inference_prompt and at least one agent.statuses entry are required".to_string())
    )]
    fn enabled_codex_inference_rejects_incomplete_configuration(
        #[case] invalid: InvalidCodexInferenceConfig,
        #[case] expected_error: std::result::Result<(), String>,
        mut valid_codex_inference_agent_config: AgentConfig,
    ) {
        let config = &mut valid_codex_inference_agent_config;
        match invalid {
            InvalidCodexInferenceConfig::MissingFailureStatus => {
                config.codex_status_inference.failure_status = None;
            }
            InvalidCodexInferenceConfig::MissingBackgroundStatus => {
                config.codex_status_inference.background_status = None;
            }
            InvalidCodexInferenceConfig::UnknownKind => {
                if let Some(failure_status) = &mut config.codex_status_inference.failure_status {
                    failure_status.kind = "unknown_kind".to_string();
                }
            }
            InvalidCodexInferenceConfig::EmptyKind => {
                if let Some(failure_status) = &mut config.codex_status_inference.failure_status {
                    failure_status.kind = "  ".to_string();
                }
            }
            InvalidCodexInferenceConfig::MultilineNote => {
                if let Some(failure_status) = &mut config.codex_status_inference.failure_status {
                    failure_status.note = "first line\nsecond line".to_string();
                }
            }
            InvalidCodexInferenceConfig::UnicodeLineSeparator => {
                if let Some(failure_status) = &mut config.codex_status_inference.failure_status {
                    failure_status.note = "first second".to_string();
                }
            }
            InvalidCodexInferenceConfig::MissingPrompt => {
                config.status_inference_prompt = None;
            }
        }

        assert_eq!(
            (
                config.validate(),
                config.configured_codex_status_inference().is_some(),
            ),
            (expected_error, false),
        );
    }

    #[test]
    fn load_config_rejects_invalid_codex_inference_enablement() {
        let config_home = TempDir::new().unwrap();
        let config_dir = config_home.path().join("armyknife");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("config.yaml"),
            indoc! {"
                agent:
                  status_inference_prompt: Classify the stopped work.
                  statuses:
                    follow_up_needed: { state: user }
                  codex_status_inference:
                    enabled: true
                    failure_status: { kind: follow_up_needed, note: investigate failure }
            "},
        )
        .unwrap();
        let config_home = config_home.path().to_str().unwrap().to_string();

        let error = temp_env::with_vars(
            [
                ("XDG_CONFIG_HOME", Some(config_home.as_str())),
                (
                    "ARMYKNIFE_AGENT__CODEX_STATUS_INFERENCE__ENABLED",
                    None::<&str>,
                ),
            ],
            load_config,
        )
        .expect_err("missing background status should reject Codex inference enablement");

        assert_eq!(
            format!("{error:#}"),
            "Invalid agent configuration: agent.codex_status_inference.background_status is required when Codex inference is enabled",
        );
    }

    #[test]
    fn parse_agent_bg_task_reminder_yaml() {
        let yaml = indoc! {"
            agent:
              bg_task_reminder:
                timeout: 7m
                message: Check the long-running task.
                excluded_commands:
                  - sample-review --wait
                  - release approve
        "};
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(
            config.agent.bg_task_reminder,
            BgTaskReminderConfig {
                timeout: Some("7m".to_string()),
                message: "Check the long-running task.".to_string(),
                excluded_commands: vec![
                    "sample-review --wait".to_string(),
                    "release approve".to_string(),
                ],
            }
        );
    }

    #[test]
    fn parse_agent_without_work_types_defaults_to_empty_mapping() {
        let config: Config = serde_yaml::from_str("agent: {}\n").unwrap();

        assert_eq!(
            (
                config.agent.work_types,
                config.agent.work_type_fallback,
                config.agent.vibe_marker,
            ),
            (HashMap::new(), None, None)
        );
    }

    #[test]
    fn parse_agent_vibe_marker_yaml() {
        let yaml = indoc! {"
            agent:
              vibe_marker:
                icon: ◈
                color: [12, 34, 56]
        "};
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(
            config.agent.vibe_marker,
            Some(AgentWorkTypeConfig {
                icon: "◈".to_string(),
                color: AgentWorkTypeColor::Rgb([12, 34, 56]),
            })
        );
    }

    #[test]
    fn parse_agent_work_type_display_yaml() {
        let yaml = indoc! {"
            agent:
              work_type_fallback:
                icon: ∅
                color: yellow
              work_types:
                sample-flow:
                  icon: ◇
                  color: cyan
                rgb-flow:
                  icon: ◆
                  color: [12, 34, 56]
                indexed-flow:
                  icon: ○
                  color: 123
        "};
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(
            (config.agent.work_type_fallback, config.agent.work_types,),
            (
                Some(AgentWorkTypeConfig {
                    icon: "∅".to_string(),
                    color: AgentWorkTypeColor::Named(AgentWorkTypeNamedColor::Yellow),
                }),
                HashMap::from([
                    (
                        "sample-flow".to_string(),
                        AgentWorkTypeConfig {
                            icon: "◇".to_string(),
                            color: AgentWorkTypeColor::Named(AgentWorkTypeNamedColor::Cyan),
                        },
                    ),
                    (
                        "rgb-flow".to_string(),
                        AgentWorkTypeConfig {
                            icon: "◆".to_string(),
                            color: AgentWorkTypeColor::Rgb([12, 34, 56]),
                        },
                    ),
                    (
                        "indexed-flow".to_string(),
                        AgentWorkTypeConfig {
                            icon: "○".to_string(),
                            color: AgentWorkTypeColor::Indexed(123),
                        },
                    ),
                ])
            )
        );
    }

    #[rstest]
    #[case::configured(
        "sample-flow",
        Some((
            "◇",
            AgentWorkTypeColor::Named(AgentWorkTypeNamedColor::Cyan)
        ))
    )]
    #[case::unconfigured("unknown-flow", None)]
    #[case::default_named_skill(
        "default",
        Some(("∅", AgentWorkTypeColor::Named(AgentWorkTypeNamedColor::Yellow)))
    )]
    fn agent_work_type_looks_up_by_skill_name(
        #[case] skill_name: &str,
        #[case] expected: Option<(&str, AgentWorkTypeColor)>,
    ) {
        let config = AgentConfig {
            work_types: HashMap::from([
                (
                    "sample-flow".to_string(),
                    AgentWorkTypeConfig {
                        icon: "◇".to_string(),
                        color: AgentWorkTypeColor::Named(AgentWorkTypeNamedColor::Cyan),
                    },
                ),
                (
                    "default".to_string(),
                    AgentWorkTypeConfig {
                        icon: "∅".to_string(),
                        color: AgentWorkTypeColor::Named(AgentWorkTypeNamedColor::Yellow),
                    },
                ),
            ]),
            ..Default::default()
        };
        let actual = config
            .work_type(skill_name)
            .map(|work_type| (work_type.icon.as_str(), work_type.color));

        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case::misspelled_effort("maxx")]
    #[case::x_high_spelling("x-high")]
    fn parse_agent_codex_rejects_unknown_reasoning_effort(#[case] effort: &str) {
        let yaml = formatdoc! {"
            agent:
              codex:
                reasoning_effort: {effort}
        "};
        assert!(serde_yaml::from_str::<Config>(&yaml).is_err());
    }

    #[test]
    fn parse_full_yaml_config() {
        let yaml = indoc! {"
            agent:
              worktree:
                dir: .wt
                branch_prefix: user/
                layout:
                  direction: vertical
                  first:
                    command: vim
                    focus: true
                  second:
                    command: bash
                repos_root: /example/repos
            editor:
              terminal: ghostty
              editor_command: vim
              focus_app: Alacritty
            notification:
              enabled: false
              sound: Ping
        "};
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(
            config,
            Config {
                agent: AgentConfig {
                    worktree: WorktreeConfig {
                        dir: ".wt".to_string(),
                        branch_prefix: "user/".to_string(),
                        layout: LayoutNode::Split(SplitConfig {
                            direction: SplitDirection::Vertical,
                            first: Box::new(LayoutNode::Pane(PaneConfig {
                                command: "vim".to_string(),
                                focus: true,
                            })),
                            second: Box::new(LayoutNode::Pane(PaneConfig {
                                command: "bash".to_string(),
                                focus: false,
                            })),
                        }),
                        repos_root: Some("/example/repos".to_string()),
                    },
                    ..Default::default()
                },
                editor: EditorConfig {
                    terminal: Terminal::Ghostty,
                    editor_command: "vim".to_string(),
                    focus_app: Some("Alacritty".to_string()),
                },
                notification: NotificationConfig {
                    enabled: false,
                    sound: "Ping".to_string(),
                },
                ..Default::default()
            }
        );
    }

    #[rstest]
    #[case::wm(indoc! {"wm: {}"}, "wm")]
    #[case::cc(indoc! {"cc: {}"}, "cc")]
    fn parse_legacy_config_sections_reject_unknown_fields(#[case] yaml: &str, #[case] key: &str) {
        let value: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        let error = serde_yaml::from_value::<Config>(value).unwrap_err();

        assert_eq!(
            error.to_string(),
            format!(
                "unknown field `{key}`, expected one of `agent`, `editor`, `notification`, `repos`, `orgs`"
            )
        );
    }

    #[test]
    fn parse_empty_yaml_uses_all_defaults() {
        let yaml = "{}";
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config, Config::default());
    }

    #[rstest]
    #[case::wezterm("wezterm", Terminal::Wezterm)]
    #[case::ghostty("ghostty", Terminal::Ghostty)]
    fn parse_terminal_enum(#[case] yaml_value: &str, #[case] expected: Terminal) {
        let yaml = format!("editor:\n  terminal: {}", yaml_value);
        let config: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config.editor.terminal, expected);
    }

    #[test]
    fn terminal_default_is_wezterm() {
        assert_eq!(Terminal::default(), Terminal::Wezterm);
    }

    #[rstest]
    #[case::wezterm(Terminal::Wezterm, "WezTerm")]
    #[case::ghostty(Terminal::Ghostty, "Ghostty")]
    fn terminal_default_focus_app(#[case] terminal: Terminal, #[case] expected: &str) {
        assert_eq!(terminal.default_focus_app(), expected);
    }

    #[test]
    fn editor_config_focus_app_uses_explicit_value() {
        let config = EditorConfig {
            terminal: Terminal::Ghostty,
            focus_app: Some("CustomApp".to_string()),
            ..Default::default()
        };
        assert_eq!(config.focus_app(), "CustomApp");
    }

    #[test]
    fn editor_config_focus_app_derives_from_terminal() {
        let config = EditorConfig {
            terminal: Terminal::Ghostty,
            focus_app: None,
            ..Default::default()
        };
        assert_eq!(config.focus_app(), "Ghostty");
    }

    #[test]
    fn layout_single_pane() {
        let yaml = indoc! {"
            command: nvim
            focus: true
        "};
        let layout: LayoutNode = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            layout,
            LayoutNode::Pane(PaneConfig {
                command: "nvim".to_string(),
                focus: true,
            })
        );
    }

    #[test]
    fn layout_horizontal_split() {
        let yaml = indoc! {"
            direction: horizontal
            first:
              command: nvim
              focus: true
            second:
              command: claude
        "};
        let layout: LayoutNode = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            layout,
            LayoutNode::Split(SplitConfig {
                direction: SplitDirection::Horizontal,
                first: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "nvim".to_string(),
                    focus: true,
                })),
                second: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "claude".to_string(),
                    focus: false,
                })),
            })
        );
    }

    #[test]
    fn layout_vertical_split() {
        let yaml = indoc! {"
            direction: vertical
            first:
              command: top
            second:
              command: bash
        "};
        let layout: LayoutNode = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            layout,
            LayoutNode::Split(SplitConfig {
                direction: SplitDirection::Vertical,
                first: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "top".to_string(),
                    focus: false,
                })),
                second: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "bash".to_string(),
                    focus: false,
                })),
            })
        );
    }

    #[test]
    fn layout_nested_split() {
        let yaml = indoc! {"
            direction: horizontal
            first:
              command: nvim
              focus: true
            second:
              direction: vertical
              first:
                command: claude
              second:
                command: bash
        "};
        let layout: LayoutNode = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            layout,
            LayoutNode::Split(SplitConfig {
                direction: SplitDirection::Horizontal,
                first: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "nvim".to_string(),
                    focus: true,
                })),
                second: Box::new(LayoutNode::Split(SplitConfig {
                    direction: SplitDirection::Vertical,
                    first: Box::new(LayoutNode::Pane(PaneConfig {
                        command: "claude".to_string(),
                        focus: false,
                    })),
                    second: Box::new(LayoutNode::Pane(PaneConfig {
                        command: "bash".to_string(),
                        focus: false,
                    })),
                })),
            })
        );
    }

    #[rstest]
    #[case::with_language("fohte/t-rader", "repos:\n  fohte/t-rader:\n    language: ja\n", Some("ja".to_string()))]
    #[case::without_language("fohte/some-repo", "repos:\n  fohte/some-repo: {}\n", None)]
    fn parse_repos_config(
        #[case] repo_id: &str,
        #[case] yaml: &str,
        #[case] expected_language: Option<String>,
    ) {
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.repos.len(), 1);
        assert_eq!(config.repos[repo_id].language, expected_language);
    }

    #[rstest]
    #[case::allowed(
        "fohte/dotfiles",
        indoc! {"
            repos:
              fohte/dotfiles:
                direct_commit: true
        "},
        true
    )]
    #[case::denied(
        "fohte/some-repo",
        indoc! {"
            repos:
              fohte/some-repo:
                direct_commit: false
        "},
        false
    )]
    #[case::unset_defaults_to_false(
        "fohte/another-repo",
        indoc! {"
            repos:
              fohte/another-repo: {}
        "},
        false
    )]
    fn parse_repos_config_direct_commit(
        #[case] repo_id: &str,
        #[case] yaml: &str,
        #[case] expected: bool,
    ) {
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.repos.len(), 1);
        assert_eq!(config.repos[repo_id].direct_commit, expected);
    }

    #[rstest]
    #[case::enabled(
        "example-org/standalone-tool",
        indoc! {"
            repos:
              example-org/standalone-tool:
                standalone_fork: true
        "},
        true
    )]
    #[case::disabled(
        "example-org/managed-fork",
        indoc! {"
            repos:
              example-org/managed-fork:
                standalone_fork: false
        "},
        false
    )]
    #[case::unset_defaults_to_false(
        "example-org/unset-fork",
        indoc! {"
            repos:
              example-org/unset-fork: {}
        "},
        false
    )]
    fn parse_repos_config_standalone_fork(
        #[case] repo_id: &str,
        #[case] yaml: &str,
        #[case] expected: bool,
    ) {
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(
            config.repos[repo_id],
            RepoConfig {
                standalone_fork: expected,
                ..RepoConfig::default()
            }
        );
    }

    #[test]
    fn repos_config_denies_unknown_fields() {
        let yaml = indoc! {"
            repos:
              fohte/repo:
                unknown_key: value
        "};
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown field"));
    }

    #[rstest]
    #[case::legacy_wm(indoc! {"wm: {}"}, "unknown field")]
    #[case::legacy_cc(indoc! {"cc: {}"}, "unknown field")]
    #[case::editor(indoc! {"
        editor:
          bad_field: value
    "}, "unknown field")]
    #[case::notification(indoc! {"
        notification:
          extra: true
    "}, "unknown field")]
    #[case::unknown_section(indoc! {"unknown_section: {}"}, "unknown field")]
    fn deny_unknown_fields(#[case] yaml: &str, #[case] expected_error: &str) {
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains(expected_error),
            "expected error containing '{}', got: {}",
            expected_error,
            err
        );
    }

    #[rstest]
    #[case::yaml("config.yaml")]
    #[case::yml("config.yml")]
    fn load_config_from_dir_with_config_file(#[case] filename: &str) {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(filename),
            indoc! {"
                notification:
                  enabled: false
                  sound: Ping
            "},
        )
        .unwrap();

        let config = load_config_from_dir(dir.path()).unwrap();
        assert!(!config.notification.enabled);
        assert_eq!(config.notification.sound, "Ping");
    }

    #[test]
    fn load_config_from_dir_alphabetical_later_wins() {
        // Files are merged in alphabetical order so config.yml (later) overrides
        // config.yaml (earlier) for the same key.
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.yaml"),
            indoc! {"
                notification:
                  sound: FromYaml
            "},
        )
        .unwrap();
        fs::write(
            dir.path().join("config.yml"),
            indoc! {"
                notification:
                  sound: FromYml
            "},
        )
        .unwrap();

        let config = load_config_from_dir(dir.path()).unwrap();
        assert_eq!(config.notification.sound, "FromYml");
    }

    #[rstest]
    #[case::no_file(None)]
    #[case::empty_file(Some(""))]
    #[case::comment_only(Some("# just a comment\n"))]
    fn load_config_from_dir_returns_default_for_absent_or_empty(#[case] content: Option<&str>) {
        let dir = TempDir::new().unwrap();
        if let Some(c) = content {
            fs::write(dir.path().join("config.yaml"), c).unwrap();
        }
        let config = load_config_from_dir(dir.path()).unwrap();
        assert_eq!(config, Config::default());
    }

    #[rstest]
    // Per-file YAML syntax errors blame the offending file.
    #[case::syntax_error(indoc! {"
        notification:
          enabled: [broken
    "}, true)]
    #[case::legacy_wm(indoc! {"wm: {}"}, false)]
    #[case::legacy_cc(indoc! {"cc: {}"}, false)]
    // Merged-document level errors (e.g., unknown fields) blame the directory
    // because they only surface after combining every file.
    #[case::unknown_field("unknown_top_level_key: true\n", false)]
    fn load_config_from_dir_parse_error(#[case] yaml: &str, #[case] blame_file: bool) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        fs::write(&path, yaml).unwrap();

        let err = load_config_from_dir(dir.path()).unwrap_err();
        let config_err = err.downcast_ref::<ConfigError>().unwrap();
        match config_err {
            ConfigError::ParseError {
                path: err_path,
                message,
            } => {
                let expected = if blame_file { &path } else { dir.path() };
                assert_eq!(err_path, expected);
                assert!(!message.is_empty(), "error message should not be empty");
            }
            other => panic!("expected ParseError, got: {other:?}"),
        }
    }

    #[test]
    fn stale_active_profile_does_not_block_legacy_launch_resolution() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.yaml"),
            indoc! {"
                agent:
                  active_profile: removed-profile
                  default_engine: codex
                  profiles:
                    configured-profile: {}
            "},
        )
        .unwrap();

        let config = load_config_from_dir(dir.path()).unwrap();

        assert_eq!(
            (
                config.agent.active_profile.as_deref(),
                config.agent.resolve_launch_options(None, None, None, None),
            ),
            (
                Some("removed-profile"),
                ResolvedLaunchOptions {
                    engine: Engine::Codex,
                    model: None,
                    reasoning_effort: None,
                },
            ),
        );
    }

    #[test]
    fn load_config_from_dir_partial_config_uses_defaults() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.yaml"),
            indoc! {"
                agent:
                  worktree:
                    dir: custom-wt
            "},
        )
        .unwrap();

        let config = load_config_from_dir(dir.path()).unwrap();
        assert_eq!(config.agent.worktree.dir, "custom-wt");
        // Other worktree fields use defaults
        assert_eq!(config.agent.worktree.branch_prefix, "fohte/");
        // Other sections use defaults entirely
        assert_eq!(config.editor, EditorConfig::default());
        assert_eq!(config.notification, NotificationConfig::default());
    }

    #[rstest]
    #[case::scalar_overrides_scalar("a: 1\n", "a: 2\n", "a: 2\n")]
    #[case::sequence_replaces_sequence("a: [1, 2]\n", "a: [3]\n", "a: [3]\n")]
    #[case::mapping_merges_recursively(
        "a:\n  b: 1\n  c: 2\n",
        "a:\n  c: 99\n  d: 4\n",
        "a:\n  b: 1\n  c: 99\n  d: 4\n"
    )]
    #[case::scalar_overrides_mapping("a:\n  b: 1\n", "a: leaf\n", "a: leaf\n")]
    fn merge_yaml_combines_documents(
        #[case] base: &str,
        #[case] overlay: &str,
        #[case] expected: &str,
    ) {
        let base: serde_yaml::Value = serde_yaml::from_str(base).unwrap();
        let overlay: serde_yaml::Value = serde_yaml::from_str(overlay).unwrap();
        let expected: serde_yaml::Value = serde_yaml::from_str(expected).unwrap();
        assert_eq!(merge_yaml(base, overlay), expected);
    }

    #[test]
    fn load_config_from_dir_merges_multiple_yaml_files() {
        let dir = TempDir::new().unwrap();
        // Earlier file: org defaults across two orgs.
        fs::write(
            dir.path().join("base.yaml"),
            indoc! {"
                orgs:
                  fohte:
                    ai:
                      review:
                        reviewers: [coderabbit, devin]
                  acme:
                    ai:
                      review:
                        reviewers: [devin]
            "},
        )
        .unwrap();
        // Later file: override fohte (sequence is replaced wholesale, not concatenated)
        // and add a new org.
        fs::write(
            dir.path().join("work.yaml"),
            indoc! {"
                orgs:
                  fohte:
                    ai:
                      review:
                        reviewers: [coderabbit]
                  contoso:
                    ai:
                      review:
                        reviewers: [devin]
            "},
        )
        .unwrap();

        let config = load_config_from_dir(dir.path()).unwrap();
        assert_eq!(
            config.orgs["fohte"].ai.review.reviewers,
            Some(vec![Reviewer::CodeRabbit])
        );
        assert_eq!(
            config.orgs["acme"].ai.review.reviewers,
            Some(vec![Reviewer::Devin])
        );
        assert_eq!(
            config.orgs["contoso"].ai.review.reviewers,
            Some(vec![Reviewer::Devin])
        );
    }

    #[test]
    fn load_config_from_dir_ignores_subdirectories_and_other_extensions() {
        // Only flat *.yaml/*.yml are read. A nested directory like `hooks/` and
        // unrelated files (README, .json) must be skipped.
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("hooks")).unwrap();
        fs::write(
            dir.path().join("hooks").join("post-worktree-create.yaml"),
            "should: be ignored\n",
        )
        .unwrap();
        fs::write(dir.path().join("README.md"), "ignored\n").unwrap();
        fs::write(dir.path().join("config.json"), "{}\n").unwrap();
        fs::write(
            dir.path().join("config.yaml"),
            indoc! {"
                agent:
                  worktree:
                    dir: kept
            "},
        )
        .unwrap();

        let config = load_config_from_dir(dir.path()).unwrap();
        assert_eq!(config.agent.worktree.dir, "kept");
    }

    #[test]
    fn load_config_from_dir_follows_symlinks_to_yaml_files() {
        // Symlinks are how a private repo can ship company config without
        // touching the public dotfiles tree, so they must be honored.
        let dir = TempDir::new().unwrap();
        let external = TempDir::new().unwrap();
        let target = external.path().join("work.yaml");
        fs::write(
            &target,
            indoc! {"
                orgs:
                  acme:
                    ai:
                      review:
                        reviewers: [coderabbit]
            "},
        )
        .unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("work.yaml")).unwrap();

        let config = load_config_from_dir(dir.path()).unwrap();
        assert_eq!(
            config.orgs["acme"].ai.review.reviewers,
            Some(vec![Reviewer::CodeRabbit])
        );
    }

    #[test]
    fn load_config_from_dir_returns_default_for_missing_directory() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("nonexistent");
        let config = load_config_from_dir(&missing).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn load_config_env_overlay_overrides_yaml_and_interprets_scalars() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("armyknife");
        fs::create_dir(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.yaml"),
            indoc! {"
                agent:
                  auto_compact:
                    enabled: true
                    idle_timeout: 3m
                    min_context_tokens: 100000
            "},
        )
        .unwrap();

        let config = config_with_env_vars(
            &config_dir,
            &[
                ("ARMYKNIFE_AGENT__AUTO_COMPACT__ENABLED", "false"),
                ("ARMYKNIFE_AGENT__AUTO_COMPACT__IDLE_TIMEOUT", "5m"),
                ("ARMYKNIFE_AGENT__AUTO_COMPACT__MIN_CONTEXT_TOKENS", "42"),
            ],
        )
        .unwrap();

        assert_eq!(
            config,
            Config {
                agent: AgentConfig {
                    auto_compact: AutoCompactConfig {
                        enabled: false,
                        idle_timeout: "5m".to_string(),
                        min_context_tokens: 42,
                    },
                    ..Default::default()
                },
                ..Default::default()
            }
        );
    }

    #[test]
    fn load_config_env_overlay_overrides_default_engine() {
        let dir = TempDir::new().unwrap();
        // dir/armyknife is never created: no YAML sets agent.default_engine either.

        let config =
            config_with_env_vars(dir.path(), &[("ARMYKNIFE_AGENT__DEFAULT_ENGINE", "codex")])
                .unwrap();

        assert_eq!(
            config,
            Config {
                agent: AgentConfig {
                    default_engine: Engine::Codex,
                    ..Default::default()
                },
                ..Default::default()
            }
        );
    }

    #[rstest]
    fn load_config_env_overlay_selects_profile_for_typed_launch(profile_config_dir: TempDir) {
        let config = config_with_env_vars(
            profile_config_dir.path(),
            &[("ARMYKNIFE_AGENT__ACTIVE_PROFILE", "selected-profile")],
        )
        .unwrap();

        assert_eq!(
            config.agent.resolve_profile_type("review-type").unwrap(),
            ResolvedAgentProfileType {
                profile_name: "selected-profile".to_string(),
                type_name: "review-type".to_string(),
                settings: AgentProfileTypeConfig {
                    engine: AgentProfileTypeEngine::Codex,
                    model: Some("model-ember".to_string()),
                    reasoning_effort: Some(ReasoningEffort::High),
                },
            },
        );
    }

    #[rstest]
    fn load_config_env_overlay_reports_undefined_profile_when_resolving_type(
        profile_config_dir: TempDir,
    ) {
        let config = config_with_env_vars(
            profile_config_dir.path(),
            &[("ARMYKNIFE_AGENT__ACTIVE_PROFILE", "missing-profile")],
        )
        .unwrap();
        let error = config
            .agent
            .resolve_profile_type("review-type")
            .err()
            .map(|error| error.to_string());

        assert_eq!(
            error,
            Some("agent.active_profile references undefined profile `missing-profile`".to_string()),
        );
    }

    #[rstest]
    #[case::wm("ARMYKNIFE_WM__WORKTREES_DIR", "wm")]
    #[case::cc("ARMYKNIFE_CC__AUTO_PAUSE__TIMEOUT", "cc")]
    fn load_config_rejects_legacy_env_sections(#[case] name: &str, #[case] key: &str) {
        let dir = TempDir::new().unwrap();

        let error = config_with_env_vars(dir.path(), &[(name, "30m")]).unwrap_err();
        let config_error = error.downcast_ref::<ConfigError>().unwrap();
        let expected = format!(
            "unknown field `{key}`, expected one of `agent`, `editor`, `notification`, `repos`, `orgs`"
        );
        match config_error {
            ConfigError::EnvParseError { message } => assert_eq!(message, &expected),
            other => panic!("expected EnvParseError, got: {other:?}"),
        }
    }

    #[test]
    fn load_config_env_overlay_works_without_any_yaml_config() {
        let dir = TempDir::new().unwrap();
        // dir/armyknife is never created, so there's no config directory at all.

        let config =
            config_with_env_vars(dir.path(), &[("ARMYKNIFE_NOTIFICATION__SOUND", "Ping")]).unwrap();

        assert_eq!(
            config,
            Config {
                notification: NotificationConfig {
                    sound: "Ping".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            }
        );
    }

    #[test]
    fn load_config_unknown_env_key_errors_and_blames_environment() {
        let dir = TempDir::new().unwrap();

        let err = config_with_env_vars(dir.path(), &[("ARMYKNIFE_AGENT__WORKTREE__TYPO", "1")])
            .unwrap_err();

        let config_err = err.downcast_ref::<ConfigError>().unwrap();
        match config_err {
            ConfigError::EnvParseError { message } => {
                assert_eq!(
                    message,
                    "unknown field `typo`, expected one of `dir`, `branch_prefix`, `layout`, `repos_root`"
                );
            }
            other => panic!("expected EnvParseError, got: {other:?}"),
        }
    }

    #[rstest]
    #[case::repo_beats_org("fohte", "work-repo", Some(vec![Reviewer::Devin]))]
    #[case::org_when_no_repo_entry("fohte", "any-repo", Some(vec![Reviewer::CodeRabbit]))]
    // Guards the inner `let`-chain: a repo entry that exists but leaves
    // `ai.review.reviewers` unset must still fall through to the org default.
    #[case::repo_entry_without_reviewers_falls_back_to_org("fohte", "no-reviewers", Some(vec![Reviewer::CodeRabbit]))]
    #[case::owner_unknown_returns_none("stranger", "any-repo", None)]
    fn resolve_reviewers_precedence(
        #[case] owner: &str,
        #[case] repo: &str,
        #[case] expected: Option<Vec<Reviewer>>,
    ) {
        let config: Config = serde_yaml::from_str(indoc! {"
            orgs:
              fohte:
                ai:
                  review:
                    reviewers: [coderabbit]
            repos:
              fohte/work-repo:
                ai:
                  review:
                    reviewers: [devin]
              fohte/no-reviewers:
                ai:
                  review: {}
        "})
        .unwrap();

        assert_eq!(config.resolve_reviewers(owner, repo), expected);
    }

    #[cfg(feature = "schema-gen")]
    #[fixture]
    fn schema_value() -> serde_json::Value {
        let schema = generate_schema();
        serde_json::to_value(&schema).unwrap()
    }

    #[cfg(feature = "schema-gen")]
    #[rstest]
    fn generate_schema_returns_valid_json_with_title(schema_value: serde_json::Value) {
        // schemars generates a title from the struct name
        assert_eq!(schema_value["title"], "Config");
    }

    #[cfg(feature = "schema-gen")]
    #[rstest]
    fn generate_schema_has_current_top_level_config_keys(schema_value: serde_json::Value) {
        let runtime_value = serde_json::to_value(Config::default()).unwrap();
        let runtime_keys = runtime_value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let schema_keys = schema_value["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(schema_keys, runtime_keys);
    }

    #[cfg(feature = "schema-gen")]
    #[rstest]
    fn generate_schema_contains_agent_description(schema_value: serde_json::Value) {
        // Doc comments should appear as descriptions in the schema
        assert_eq!(
            schema_value["properties"]["agent"]["description"],
            "`a agent` settings."
        );
    }

    #[cfg(feature = "schema-gen")]
    #[rstest]
    fn generate_schema_contains_merge_notification_description(schema_value: serde_json::Value) {
        assert_eq!(
            schema_value["$defs"]["AgentConfig"]["properties"]["merge_notification"],
            serde_json::json!({
                "description": indoc! {"
                    Message template sent to the delegator when a delegated branch's PR merges.
                    Supports `{branch}` and `{pr_url}` placeholders. When unset, no notification is sent."},
                "type": ["string", "null"],
                "default": null,
            })
        );
    }

    #[cfg(feature = "schema-gen")]
    #[rstest]
    fn generate_schema_contains_stop_hook_instruction_description(schema_value: serde_json::Value) {
        assert_eq!(
            schema_value["$defs"]["AgentConfig"]["properties"]["stop_hook_instruction"],
            serde_json::json!({
                "description": indoc! {"
                    Instruction returned by the Stop hook when it blocks a session.
                    Codex sessions use this instruction unless Codex status inference is
                    enabled; Claude sessions use it when status inference is not configured.
                    At least one status must be configured."},
                "type": ["string", "null"],
                "default": null,
            })
        );
    }

    #[cfg(feature = "schema-gen")]
    #[rstest]
    fn generate_schema_contains_status_inference_prompt_description(
        schema_value: serde_json::Value,
    ) {
        assert_eq!(
            schema_value["$defs"]["AgentConfig"]["properties"]["status_inference_prompt"],
            serde_json::json!({
                "description": indoc! {"
                    Prompt used by a Claude Code fork or Codex fork to classify a missing
                    status at Stop."},
                "type": ["string", "null"],
                "default": null,
            })
        );
    }

    #[cfg(feature = "schema-gen")]
    #[rstest]
    fn generate_schema_describes_codex_status_inference_settings(schema_value: serde_json::Value) {
        assert_eq!(
            schema_value["$defs"]["CodexStatusInferenceConfig"],
            serde_json::json!({
                "description": "Opt-in settings for inferring Codex Stop status with a fork of the saved conversation. Inference requires the built-in OpenAI provider; enabling requires a non-empty `agent.status_inference_prompt` and at least one `agent.statuses` entry.",
                "type": "object",
                "properties": {
                    "background_status": {
                        "description": "Status recorded while armyknife background work is still pending.",
                        "anyOf": [
                            { "$ref": "#/$defs/StatusInferenceStatusConfig" },
                            { "type": "null" },
                        ],
                        "default": null,
                    },
                    "enabled": {
                        "description": "Enable Codex Stop inference. When enabled, failure and background statuses are required.",
                        "type": "boolean",
                        "default": false,
                    },
                    "failure_status": {
                        "description": "Status recorded if snapshot acquisition, the Codex fork, classification, validation, or worker startup fails.",
                        "anyOf": [
                            { "$ref": "#/$defs/StatusInferenceStatusConfig" },
                            { "type": "null" },
                        ],
                        "default": null,
                    },
                    "shadow_mode": {
                        "description": "Log classifications without writing any status to the session database.",
                        "type": "boolean",
                        "default": false,
                    },
                },
                "additionalProperties": false,
            })
        );
    }

    #[cfg(feature = "schema-gen")]
    #[rstest]
    fn generate_schema_contains_bg_task_reminder_settings(schema_value: serde_json::Value) {
        let properties = &schema_value["$defs"]["BgTaskReminderConfig"]["properties"];

        assert_eq!(
            (
                properties["timeout"]["type"].clone(),
                properties["message"]["default"].clone(),
                properties["excluded_commands"]["default"].clone(),
            ),
            (
                serde_json::json!(["string", "null"]),
                serde_json::json!(
                    "This is a reminder, not an error: this background task has been running for a long time. Check its output and the state of what it is monitoring. Stop it if it is stuck."
                ),
                serde_json::json!([]),
            )
        );
    }

    #[cfg(feature = "schema-gen")]
    #[rstest]
    fn generate_schema_contains_default_values(schema_value: serde_json::Value) {
        // Default values from schemars(default = ...) should appear in the schema.
        let defs = &schema_value["$defs"];
        let worktree_defaults = &defs["WorktreeConfig"]["properties"];
        assert_eq!(
            (
                worktree_defaults["dir"]["default"].clone(),
                worktree_defaults["branch_prefix"]["default"].clone(),
            ),
            (serde_json::json!(".worktrees"), serde_json::json!("fohte/"),)
        );

        let notification_defaults = &defs["NotificationConfig"]["properties"];
        let editor_defaults = &defs["EditorConfig"]["properties"];
        let terminal_def = &defs["Terminal"];
        assert_eq!(
            (
                notification_defaults["sound"]["default"].clone(),
                editor_defaults["editor_command"]["default"].clone(),
                terminal_def["oneOf"].is_array() || terminal_def["enum"].is_array(),
            ),
            (serde_json::json!("Glass"), serde_json::json!("nvim"), true,)
        );
    }
}
