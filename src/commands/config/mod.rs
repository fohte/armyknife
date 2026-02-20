use std::path::PathBuf;
use std::process;

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

    /// Get a configuration value by key
    Get {
        /// Configuration key (e.g., "repo.language")
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
    if let Some(repo_key) = key.strip_prefix("repo.") {
        let cfg = config::load_config()?;
        let Some((owner, repo)) = git::get_owner_repo() else {
            // Not in a git repo or no origin remote
            process::exit(1);
        };
        let repo_id = format!("{owner}/{repo}");

        if let Some(repo_config) = cfg.repos.get(&repo_id) {
            let value = match repo_key {
                "language" => repo_config.language.as_deref(),
                _ => {
                    eprintln!("Unknown repo config key: {repo_key}");
                    process::exit(1);
                }
            };
            if let Some(v) = value {
                println!("{v}");
            }
        }
        Ok(())
    } else {
        eprintln!("Unsupported config key: {key}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn schema_generates_valid_json() {
        let schema = crate::shared::config::generate_schema();
        let value: serde_json::Value = serde_json::to_value(&schema).unwrap();

        // schemars v1 generates a JSON Schema with "title" and "type" keys
        assert_eq!(value["title"], "Config");
        assert_eq!(value["type"], "object");
    }

    #[test]
    fn schema_contains_config_properties() {
        let schema = crate::shared::config::generate_schema();
        let value: serde_json::Value = serde_json::to_value(&schema).unwrap();

        // Verify key config sections appear as properties
        let props = value["properties"].as_object().unwrap();
        assert!(props.contains_key("wm"));
        assert!(props.contains_key("editor"));
        assert!(props.contains_key("notification"));
        assert!(props.contains_key("repos"));

        // Verify WmConfig properties exist via $defs
        let defs = value["$defs"].as_object().unwrap();
        let wm_props = defs["WmConfig"]["properties"].as_object().unwrap();
        assert!(wm_props.contains_key("worktrees_dir"));
        assert!(wm_props.contains_key("branch_prefix"));
    }
}
