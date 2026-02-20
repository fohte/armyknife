use std::path::PathBuf;

use clap::Subcommand;

use crate::infra::git;
use crate::shared::config;

/// Configuration management commands.
#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum ConfigCommands {
    /// Print JSON Schema for the configuration file
    Schema {
        /// Write schema to file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Get a configuration value by dot-separated key
    Get {
        /// Configuration key (e.g., "wm.branch_prefix", "repo.language")
        key: String,
    },
}

impl ConfigCommands {
    pub fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::Schema { output } => {
                let schema = config::generate_schema();
                let json = serde_json::to_string_pretty(&schema)?;
                if let Some(path) = output {
                    std::fs::write(path, format!("{json}\n"))?;
                    eprintln!("Schema written to {}", path.display());
                } else {
                    println!("{json}");
                }
                Ok(())
            }
            Self::Get { key } => {
                run_get(key)?;
                Ok(())
            }
        }
    }
}

fn run_get(key: &str) -> anyhow::Result<()> {
    let cfg = config::load_config()?;

    // For repo.* keys, resolve owner/repo from CWD git remote
    let repo_id = if key.starts_with("repo.") {
        git::get_owner_repo().map(|(owner, repo)| format!("{owner}/{repo}"))
    } else {
        None
    };

    if let Some(value) = cfg.get_value(key, repo_id.as_deref()) {
        println!("{value}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::shared::config::Config;
    use indoc::indoc;
    use rstest::rstest;

    fn config_from_yaml(yaml: &str) -> Config {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[rstest]
    #[case::wm_worktrees_dir("wm.worktrees_dir", ".worktrees")]
    #[case::wm_branch_prefix("wm.branch_prefix", "fohte/")]
    #[case::editor_editor_command("editor.editor_command", "nvim")]
    #[case::editor_terminal("editor.terminal", "wezterm")]
    #[case::notification_enabled("notification.enabled", "true")]
    #[case::notification_sound("notification.sound", "Glass")]
    fn get_value_returns_default_values(#[case] key: &str, #[case] expected: &str) {
        let cfg = Config::default();
        let result = cfg.get_value(key, None);
        assert_eq!(result.as_deref(), Some(expected));
    }

    #[test]
    fn get_value_returns_custom_values() {
        let cfg = config_from_yaml(indoc! {"
            wm:
              worktrees_dir: .wt
              branch_prefix: user/
            notification:
              enabled: false
              sound: Ping
        "});
        assert_eq!(
            cfg.get_value("wm.worktrees_dir", None).as_deref(),
            Some(".wt")
        );
        assert_eq!(
            cfg.get_value("wm.branch_prefix", None).as_deref(),
            Some("user/")
        );
        assert_eq!(
            cfg.get_value("notification.enabled", None).as_deref(),
            Some("false")
        );
        assert_eq!(
            cfg.get_value("notification.sound", None).as_deref(),
            Some("Ping")
        );
    }

    #[test]
    fn get_value_returns_none_for_unknown_key() {
        let cfg = Config::default();
        assert_eq!(cfg.get_value("nonexistent", None), None);
        assert_eq!(cfg.get_value("wm.nonexistent", None), None);
    }

    #[test]
    fn get_value_repo_language() {
        let cfg = config_from_yaml(indoc! {"
            repos:
              fohte/t-rader:
                language: ja
        "});
        assert_eq!(
            cfg.get_value("repo.language", Some("fohte/t-rader"))
                .as_deref(),
            Some("ja")
        );
    }

    #[test]
    fn get_value_repo_not_configured() {
        let cfg = Config::default();
        assert_eq!(cfg.get_value("repo.language", Some("fohte/unknown")), None);
    }

    #[test]
    fn get_value_repo_without_repo_id() {
        let cfg = config_from_yaml(indoc! {"
            repos:
              fohte/t-rader:
                language: ja
        "});
        assert_eq!(cfg.get_value("repo.language", None), None);
    }

    #[test]
    fn schema_generates_valid_json() {
        let schema = crate::shared::config::generate_schema();
        let value: serde_json::Value = serde_json::to_value(&schema).unwrap();

        assert_eq!(value["title"], "Config");
        assert_eq!(value["type"], "object");
    }

    #[test]
    fn schema_contains_config_properties() {
        let schema = crate::shared::config::generate_schema();
        let value: serde_json::Value = serde_json::to_value(&schema).unwrap();

        let props = value["properties"].as_object().unwrap();
        assert!(props.contains_key("wm"));
        assert!(props.contains_key("editor"));
        assert!(props.contains_key("notification"));
        assert!(props.contains_key("repos"));

        let defs = value["$defs"].as_object().unwrap();
        let wm_props = defs["WmConfig"]["properties"].as_object().unwrap();
        assert!(wm_props.contains_key("worktrees_dir"));
        assert!(wm_props.contains_key("branch_prefix"));
    }
}
