use std::collections::HashMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::commands::ai::review::reviewer::Reviewer;

/// Top-level configuration for armyknife.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Worktree management settings.
    #[serde(default)]
    pub wm: WmConfig,

    /// Terminal/editor settings for human-in-the-loop reviews.
    #[serde(default)]
    pub editor: EditorConfig,

    /// Notification settings.
    #[serde(default)]
    pub notification: NotificationConfig,

    /// Claude Code session monitoring settings.
    #[serde(default)]
    pub cc: CcConfig,

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
    /// Returns the value as a string, or None if not found.
    pub fn get_value(&self, key: &str, repo_id: Option<&str>) -> Option<String> {
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

/// Resolve a dot-separated path against a JSON value, returning a string representation.
fn resolve_json_path(value: &serde_json::Value, path: &str) -> Option<String> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    match current {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        // Non-scalar values (objects, arrays, null) are not leaf config values
        _ => None,
    }
}

/// Worktree management configuration.
#[derive(Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WmConfig {
    /// Worktree directory name (default: ".worktrees").
    #[serde(default = "default_worktrees_dir")]
    #[schemars(default = "default_worktrees_dir")]
    pub worktrees_dir: String,

    /// Branch prefix (default: "fohte/").
    #[serde(default = "default_branch_prefix")]
    #[schemars(default = "default_branch_prefix")]
    pub branch_prefix: String,

    /// tmux pane layout definition.
    #[serde(default)]
    pub layout: LayoutNode,

    /// Root directory containing git repositories.
    /// Used by `wm clean --all` to discover repositories.
    /// Falls back to GHQ_ROOT env, git config ghq.root, or ~/ghq.
    #[serde(default)]
    pub repos_root: Option<String>,
}

impl Default for WmConfig {
    fn default() -> Self {
        Self {
            worktrees_dir: default_worktrees_dir(),
            branch_prefix: default_branch_prefix(),
            layout: LayoutNode::default(),
            repos_root: None,
        }
    }
}

/// Layout tree node: either a single pane (leaf) or a split (internal node).
#[derive(Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
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
#[derive(Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
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
#[derive(Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    /// Horizontal split (tmux split-window -h).
    Horizontal,
    /// Vertical split (tmux split-window -v).
    Vertical,
}

/// Configuration for a single tmux pane.
#[derive(Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PaneConfig {
    /// Command to run in the pane.
    pub command: String,
    /// Whether to focus this pane (default: false).
    #[serde(default)]
    pub focus: bool,
}

/// Terminal emulator to use for human-in-the-loop reviews.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
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
#[derive(Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EditorConfig {
    /// Terminal emulator (default: "wezterm").
    #[serde(default)]
    pub terminal: Terminal,

    /// Editor command (default: "nvim").
    #[serde(default = "default_editor_command")]
    #[schemars(default = "default_editor_command")]
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
#[derive(Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NotificationConfig {
    /// Whether notifications are enabled (default: true).
    #[serde(default = "default_true")]
    #[schemars(default = "default_true")]
    pub enabled: bool,

    /// Notification sound name (default: "Glass"). Empty string for silent.
    #[serde(default = "default_notification_sound")]
    #[schemars(default = "default_notification_sound")]
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
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    /// Language for commit messages and PR content (e.g., "ja", "en").
    #[serde(default)]
    pub language: Option<String>,

    /// Whether direct commits to the default branch (e.g., master/main) are allowed.
    /// Consumed by external git hooks; armyknife only stores and exposes the value.
    #[serde(default)]
    #[schemars(default)]
    pub direct_commit: bool,

    /// AI-related per-repo overrides (e.g., reviewer set for `a ai review wait`).
    #[serde(default)]
    pub ai: AiConfig,
}

/// Per-organization (GitHub owner) configuration.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OrgConfig {
    /// AI-related per-org defaults (e.g., reviewer set for `a ai review wait`).
    #[serde(default)]
    pub ai: AiConfig,
}

/// Settings under the `ai` key (used in both org and repo scopes).
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiConfig {
    /// Settings for `a ai review` commands.
    #[serde(default)]
    pub review: AiReviewConfig,
}

/// Settings for `a ai review` commands.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiReviewConfig {
    /// Default reviewers for `a ai review wait` and `a ai review request`.
    /// `None` means "fall back to the next layer" (org -> built-in default).
    #[serde(default)]
    pub reviewers: Option<Vec<Reviewer>>,
}

/// Claude Code session monitoring configuration.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CcConfig {
    /// Automatic pause settings for long-stopped sessions.
    #[serde(default)]
    pub auto_pause: AutoPauseConfig,

    /// Automatic `/compact` settings for idle sessions while the prompt cache
    /// is still warm.
    #[serde(default)]
    pub auto_compact: AutoCompactConfig,
}

/// Configuration for automatically pausing sessions that stay in the Stopped
/// state for longer than `timeout`.
///
/// A periodic `a cc sweep` run (typically driven by launchd) scans all sessions,
/// sends SIGTERM to any Claude Code process whose session has been Stopped for
/// longer than `timeout`, and flips the session status to Paused so that
/// `a cc resume` can restore it later.
#[derive(Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AutoPauseConfig {
    /// Whether automatic pausing is enabled (default: true).
    #[serde(default = "default_true")]
    #[schemars(default = "default_true")]
    pub enabled: bool,

    /// How long a session must stay in Stopped before being paused.
    /// Accepts human-friendly durations parsed by the `humantime` style parser
    /// built into armyknife, e.g., "30s", "10m", "1h30m".
    /// Default: "30m".
    #[serde(default = "default_auto_pause_timeout")]
    #[schemars(default = "default_auto_pause_timeout")]
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
/// The Stop hook spawns a detached `a cc auto-compact schedule` process for
/// each Stop event. After `idle_timeout` of inactivity (anchored on the Stop
/// hook fire time), it SIGTERMs the live `claude` process and then re-runs
/// `claude -r <id> -p "/compact"` so that the compaction itself benefits from
/// the still-warm prompt cache.
///
/// Disabled by default because compaction is destructive (the active context
/// window is rewritten); users opt in once they're comfortable with the
/// trade-off.
#[derive(Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AutoCompactConfig {
    /// Whether automatic compaction is enabled (default: false).
    #[serde(default)]
    #[schemars(default)]
    pub enabled: bool,

    /// How long a session must stay idle (since the last Stop hook) before
    /// auto-compact fires. Should be slightly less than the prompt cache TTL
    /// so the resulting `/compact` invocation hits a warm cache (Claude Code
    /// subscriptions: 5m TTL → default 4m30s).
    #[serde(default = "default_auto_compact_idle_timeout")]
    #[schemars(default = "default_auto_compact_idle_timeout")]
    pub idle_timeout: String,
}

impl Default for AutoCompactConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_timeout: default_auto_compact_idle_timeout(),
        }
    }
}

fn default_auto_compact_idle_timeout() -> String {
    "4m30s".to_string()
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
}

/// Load configuration from ~/.config/armyknife/.
/// Returns Config::default() if the directory doesn't exist or contains no YAML files.
pub fn load_config() -> anyhow::Result<Config> {
    let Some(dir) = super::dirs::config_dir() else {
        return Ok(Config::default());
    };
    load_config_from_dir(&dir.join("armyknife"))
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
pub fn load_config_from_dir(dir: &Path) -> anyhow::Result<Config> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
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

    if paths.is_empty() {
        return Ok(Config::default());
    }

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

    let Some(value) = merged else {
        return Ok(Config::default());
    };

    serde_yaml::from_value(value)
        .map_err(|e| ConfigError::ParseError {
            // Use the directory path here because the error reflects the merged
            // document, not any single source file.
            path: dir.to_path_buf(),
            message: e.to_string(),
        })
        .map_err(Into::into)
}

/// Recursively merge two YAML values. Mappings merge key-by-key; any other type
/// (sequence, scalar, null) is replaced wholesale by `overlay`. This means
/// `reviewers: [gemini]` cleanly overrides a previous `reviewers: [gemini, devin]`
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

/// Generate JSON Schema for the Config struct.
pub fn generate_schema() -> schemars::Schema {
    schemars::schema_for!(Config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn config_default_has_expected_values() {
        let config = Config::default();

        assert_eq!(config.wm.worktrees_dir, ".worktrees");
        assert_eq!(config.wm.branch_prefix, "fohte/");
        assert_eq!(
            config.wm.layout,
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
        assert_eq!(config.editor.terminal, Terminal::Wezterm);
        assert_eq!(config.editor.editor_command, "nvim");
        assert_eq!(config.editor.focus_app, None);
        assert_eq!(config.editor.focus_app(), "WezTerm");
        assert!(config.notification.enabled);
        assert_eq!(config.notification.sound, "Glass");
        assert!(config.repos.is_empty());
    }

    #[test]
    fn auto_compact_default_is_disabled_with_cache_friendly_timeout() {
        let cfg = AutoCompactConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.idle_timeout, "4m30s");
    }

    #[test]
    fn parse_auto_compact_yaml() {
        let yaml = indoc! {"
            cc:
              auto_compact:
                enabled: true
                idle_timeout: 3m
        "};
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(config.cc.auto_compact.enabled);
        assert_eq!(config.cc.auto_compact.idle_timeout, "3m");
    }

    #[test]
    fn parse_full_yaml_config() {
        let yaml = indoc! {"
            wm:
              worktrees_dir: .wt
              branch_prefix: user/
              layout:
                direction: vertical
                first:
                  command: vim
                  focus: true
                second:
                  command: bash
            editor:
              terminal: ghostty
              editor_command: vim
              focus_app: Alacritty
            notification:
              enabled: false
              sound: Ping
        "};
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.wm.worktrees_dir, ".wt");
        assert_eq!(config.wm.branch_prefix, "user/");
        assert_eq!(
            config.wm.layout,
            LayoutNode::Split(SplitConfig {
                direction: SplitDirection::Vertical,
                first: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "vim".to_string(),
                    focus: true,
                })),
                second: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "bash".to_string(),
                    focus: false,
                })),
            })
        );
        assert_eq!(config.editor.terminal, Terminal::Ghostty);
        assert_eq!(config.editor.editor_command, "vim");
        assert_eq!(config.editor.focus_app, Some("Alacritty".to_string()));
        assert_eq!(config.editor.focus_app(), "Alacritty");
        assert!(!config.notification.enabled);
        assert_eq!(config.notification.sound, "Ping");
    }

    #[test]
    fn parse_partial_yaml_uses_defaults() {
        let yaml = indoc! {"
            wm:
              worktrees_dir: custom-worktrees
        "};
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.wm.worktrees_dir, "custom-worktrees");
        // Other wm fields use defaults
        assert_eq!(config.wm.branch_prefix, "fohte/");
        assert_eq!(config.wm.layout, LayoutNode::default());
        // Other sections use defaults
        assert_eq!(config.editor, EditorConfig::default());
        assert_eq!(config.notification, NotificationConfig::default());
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
    #[case("wm:\n  unknown_field: value\n", "unknown field")]
    #[case("editor:\n  bad_field: value\n", "unknown field")]
    #[case("notification:\n  extra: true\n", "unknown field")]
    #[case("unknown_section: {}\n", "unknown field")]
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
    #[case::syntax_error("wm:\n  - [broken\n", true)]
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
    fn load_config_from_dir_partial_config_uses_defaults() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.yaml"),
            indoc! {"
                wm:
                  worktrees_dir: custom-wt
            "},
        )
        .unwrap();

        let config = load_config_from_dir(dir.path()).unwrap();
        assert_eq!(config.wm.worktrees_dir, "custom-wt");
        // Other wm fields use defaults
        assert_eq!(config.wm.branch_prefix, "fohte/");
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
                        reviewers: [gemini, devin]
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
                        reviewers: [gemini]
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
            Some(vec![Reviewer::Gemini])
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
                wm:
                  worktrees_dir: kept
            "},
        )
        .unwrap();

        let config = load_config_from_dir(dir.path()).unwrap();
        assert_eq!(config.wm.worktrees_dir, "kept");
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
                        reviewers: [gemini]
            "},
        )
        .unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("work.yaml")).unwrap();

        let config = load_config_from_dir(dir.path()).unwrap();
        assert_eq!(
            config.orgs["acme"].ai.review.reviewers,
            Some(vec![Reviewer::Gemini])
        );
    }

    #[test]
    fn load_config_from_dir_returns_default_for_missing_directory() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("nonexistent");
        let config = load_config_from_dir(&missing).unwrap();
        assert_eq!(config, Config::default());
    }

    #[rstest]
    #[case::repo_beats_org("fohte", "work-repo", Some(vec![Reviewer::Devin]))]
    #[case::org_when_no_repo_entry("fohte", "any-repo", Some(vec![Reviewer::Gemini]))]
    // Guards the inner `let`-chain: a repo entry that exists but leaves
    // `ai.review.reviewers` unset must still fall through to the org default.
    #[case::repo_entry_without_reviewers_falls_back_to_org("fohte", "no-reviewers", Some(vec![Reviewer::Gemini]))]
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
                    reviewers: [gemini]
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

    #[fixture]
    fn schema_value() -> serde_json::Value {
        let schema = generate_schema();
        serde_json::to_value(&schema).unwrap()
    }

    #[rstest]
    fn generate_schema_returns_valid_json_with_title(schema_value: serde_json::Value) {
        // schemars generates a title from the struct name
        assert_eq!(schema_value["title"], "Config");
    }

    #[rstest]
    fn generate_schema_contains_wm_description(schema_value: serde_json::Value) {
        // Doc comments should appear as descriptions in the schema
        let wm_desc = schema_value["properties"]["wm"]["description"]
            .as_str()
            .unwrap_or("");
        assert_eq!(wm_desc, "Worktree management settings.");
    }

    #[rstest]
    fn generate_schema_contains_default_values(schema_value: serde_json::Value) {
        // Default values from schemars(default = ...) should appear in the schema.
        // Navigate through $ref to find WmConfig properties.
        let defs = &schema_value["$defs"];
        let wm_defaults = &defs["WmConfig"]["properties"];
        assert_eq!(wm_defaults["worktrees_dir"]["default"], ".worktrees");
        assert_eq!(wm_defaults["branch_prefix"]["default"], "fohte/");

        let notification_defaults = &defs["NotificationConfig"]["properties"];
        assert_eq!(notification_defaults["sound"]["default"], "Glass");

        let editor_defaults = &defs["EditorConfig"]["properties"];
        assert_eq!(editor_defaults["editor_command"]["default"], "nvim");

        // Terminal enum default is handled by #[default] attribute
        let terminal_def = &defs["Terminal"];
        assert!(
            terminal_def["oneOf"].is_array() || terminal_def["enum"].is_array(),
            "Terminal should be an enum in the schema"
        );
    }
}
