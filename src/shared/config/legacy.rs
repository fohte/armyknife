use std::collections::HashMap;

use serde::Deserialize;

use super::{Config, EditorConfig, NotificationConfig, OrgConfig, RepoConfig, merge_yaml};

impl<'de> Deserialize<'de> for Config {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawConfig::deserialize(deserializer)?;
        let mut legacy_agent = serde_yaml::Mapping::new();

        if let Some(worktree) = legacy_section::<D::Error>(
            "wm",
            raw.wm,
            &["worktrees_dir", "branch_prefix", "layout", "repos_root"],
            |name| if name == "worktrees_dir" { "dir" } else { name },
        )? {
            legacy_agent.insert(
                serde_yaml::Value::String("worktree".to_string()),
                serde_yaml::Value::Mapping(worktree),
            );
        }

        if let Some(cc) =
            legacy_section::<D::Error>("cc", raw.cc, &["auto_pause", "auto_compact"], |name| name)?
        {
            legacy_agent.extend(cc);
        }

        let legacy_agent = serde_yaml::Value::Mapping(legacy_agent);
        let agent_value = match raw.agent.0 {
            Some(agent) => merge_yaml(legacy_agent, agent),
            None => legacy_agent,
        };
        let agent = serde_yaml::from_value(agent_value).map_err(serde::de::Error::custom)?;

        Ok(Self {
            agent,
            editor: raw.editor,
            notification: raw.notification,
            repos: raw.repos,
            orgs: raw.orgs,
        })
    }
}

#[derive(Default)]
struct PresentYamlValue(Option<serde_yaml::Value>);

impl<'de> Deserialize<'de> for PresentYamlValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        serde_yaml::Value::deserialize(deserializer).map(|value| Self(Some(value)))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    agent: PresentYamlValue,
    #[serde(default)]
    wm: PresentYamlValue,
    #[serde(default)]
    cc: PresentYamlValue,
    #[serde(default)]
    editor: EditorConfig,
    #[serde(default)]
    notification: NotificationConfig,
    #[serde(default)]
    repos: HashMap<String, RepoConfig>,
    #[serde(default)]
    orgs: HashMap<String, OrgConfig>,
}

fn legacy_section<E>(
    section_name: &str,
    section: PresentYamlValue,
    allowed_fields: &'static [&'static str],
    rename_field: impl for<'a> Fn(&'a str) -> &'a str,
) -> Result<Option<serde_yaml::Mapping>, E>
where
    E: serde::de::Error,
{
    let Some(value) = section.0 else {
        return Ok(None);
    };
    let serde_yaml::Value::Mapping(mapping) = value else {
        return Err(E::custom(format!("`{section_name}` must be a mapping")));
    };

    let mut normalized = serde_yaml::Mapping::new();
    for (key, value) in mapping {
        let serde_yaml::Value::String(name) = key else {
            return Err(E::custom(format!("`{section_name}` keys must be strings")));
        };
        if !allowed_fields.contains(&name.as_str()) {
            return Err(E::unknown_field(&name, allowed_fields));
        }
        normalized.insert(
            serde_yaml::Value::String(rename_field(&name).to_string()),
            value,
        );
    }

    Ok(Some(normalized))
}
