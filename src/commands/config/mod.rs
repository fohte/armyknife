use clap::Subcommand;

/// Configuration management commands.
#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum ConfigCommands {
    /// Print JSON Schema for the configuration file
    Schema,
}

impl ConfigCommands {
    pub fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::Schema => {
                let schema = crate::shared::config::generate_schema();
                let json = serde_json::to_string_pretty(&schema)?;
                println!("{json}");
                Ok(())
            }
        }
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

        // Verify WmConfig properties exist via $defs
        let defs = value["$defs"].as_object().unwrap();
        let wm_props = defs["WmConfig"]["properties"].as_object().unwrap();
        assert!(wm_props.contains_key("worktrees_dir"));
        assert!(wm_props.contains_key("branch_prefix"));
    }
}
