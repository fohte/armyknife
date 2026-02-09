use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;

/// Top-level configuration for armyknife.
#[derive(Debug, Default, Deserialize, JsonSchema, PartialEq)]
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
}

/// Worktree management configuration.
#[derive(Debug, Deserialize, JsonSchema, PartialEq)]
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
}

impl Default for WmConfig {
    fn default() -> Self {
        Self {
            worktrees_dir: default_worktrees_dir(),
            branch_prefix: default_branch_prefix(),
            layout: LayoutNode::default(),
        }
    }
}

/// Layout tree node: either a single pane (leaf) or a split (internal node).
#[derive(Debug, Deserialize, JsonSchema, PartialEq)]
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
#[derive(Debug, Deserialize, JsonSchema, PartialEq)]
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
#[derive(Debug, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    /// Horizontal split (tmux split-window -h).
    Horizontal,
    /// Vertical split (tmux split-window -v).
    Vertical,
}

/// Configuration for a single tmux pane.
#[derive(Debug, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PaneConfig {
    /// Command to run in the pane.
    pub command: String,
    /// Whether to focus this pane (default: false).
    #[serde(default)]
    pub focus: bool,
}

/// Terminal emulator to use for human-in-the-loop reviews.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema, PartialEq)]
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
#[derive(Debug, Deserialize, JsonSchema, PartialEq)]
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
#[derive(Debug, Deserialize, JsonSchema, PartialEq)]
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

/// Resolve the config directory using XDG_CONFIG_HOME or ~/.config fallback.
/// CLI tools conventionally use ~/.config on all platforms (including macOS),
/// unlike `dirs::config_dir()` which returns ~/Library/Application Support on macOS.
fn config_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("armyknife"));
    }
    dirs::home_dir().map(|home| home.join(".config").join("armyknife"))
}

/// Load configuration from ~/.config/armyknife/config.ya?ml.
/// Returns Config::default() if no config file exists.
pub fn load_config() -> anyhow::Result<Config> {
    let Some(dir) = config_dir() else {
        return Ok(Config::default());
    };
    load_config_from_dir(&dir)
}

/// Load configuration from a specific directory.
/// Searches for config.yaml, then config.yml in the given directory.
/// Returns Config::default() if neither file exists.
pub fn load_config_from_dir(dir: &Path) -> anyhow::Result<Config> {
    for filename in &["config.yaml", "config.yml"] {
        let path = dir.join(filename);
        match std::fs::read_to_string(&path) {
            Ok(content) => return parse_config(&content, &path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(ConfigError::ReadError { path, source: e }.into()),
        }
    }

    Ok(Config::default())
}

/// Parse YAML content into Config.
fn parse_config(content: &str, path: &Path) -> anyhow::Result<Config> {
    // Empty or comment-only YAML deserializes as null in serde_yaml,
    // which fails for a struct. Treat as default config.
    if content.trim().is_empty()
        || serde_yaml::from_str::<serde_yaml::Value>(content).ok() == Some(serde_yaml::Value::Null)
    {
        return Ok(Config::default());
    }
    serde_yaml::from_str(content)
        .map_err(|e| ConfigError::ParseError {
            path: path.to_path_buf(),
            message: e.to_string(),
        })
        .map_err(Into::into)
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
    fn load_config_from_dir_yaml_takes_precedence_over_yml() {
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
        assert_eq!(config.notification.sound, "FromYaml");
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
    #[case::syntax_error("wm:\n  - [broken\n")]
    #[case::unknown_field("unknown_top_level_key: true\n")]
    fn load_config_from_dir_parse_error(#[case] yaml: &str) {
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
                assert_eq!(err_path, &path);
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

    #[test]
    fn config_dir_uses_xdg_config_home_when_set() {
        temp_env::with_vars([("XDG_CONFIG_HOME", Some("/custom/config"))], || {
            let dir = config_dir();
            assert_eq!(dir, Some(PathBuf::from("/custom/config/armyknife")));
        });
    }

    #[test]
    fn config_dir_falls_back_to_home_dot_config() {
        temp_env::with_vars(
            [
                ("XDG_CONFIG_HOME", None::<&str>),
                ("HOME", Some("/test/home")),
            ],
            || {
                let dir = config_dir();
                assert_eq!(dir, Some(PathBuf::from("/test/home/.config/armyknife")));
            },
        );
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
