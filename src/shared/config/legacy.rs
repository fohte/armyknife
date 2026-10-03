use serde::Deserialize;

use super::{Config, EditorConfig, NotificationConfig, OrgConfig, RepoConfig};

struct LegacyField {
    name: &'static str,
    current_name: &'static str,
}

struct LegacySection {
    name: &'static str,
    current_prefix: &'static str,
    fields: &'static [LegacyField],
}

const LEGACY_SECTIONS: &[LegacySection] = &[
    LegacySection {
        name: "wm",
        current_prefix: "agent.worktree",
        fields: &[
            LegacyField {
                name: "worktrees_dir",
                current_name: "dir",
            },
            LegacyField {
                name: "branch_prefix",
                current_name: "branch_prefix",
            },
            LegacyField {
                name: "layout",
                current_name: "layout",
            },
            LegacyField {
                name: "repos_root",
                current_name: "repos_root",
            },
        ],
    },
    LegacySection {
        name: "cc",
        current_prefix: "agent",
        fields: &[
            LegacyField {
                name: "auto_pause",
                current_name: "auto_pause",
            },
            LegacyField {
                name: "auto_compact",
                current_name: "auto_compact",
            },
        ],
    },
];

impl<'de> Deserialize<'de> for Config {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawConfig::deserialize(deserializer)?;
        let legacy_agent = legacy_agent_from_sections([("wm", raw.wm.0), ("cc", raw.cc.0)])
            .map_err(serde::de::Error::custom)?;
        let agent_value = match raw.agent.0 {
            Some(agent) => merge_legacy_yaml(serde_yaml::Value::Mapping(legacy_agent), agent),
            None => serde_yaml::Value::Mapping(legacy_agent),
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
    repos: std::collections::HashMap<String, RepoConfig>,
    #[serde(default)]
    orgs: std::collections::HashMap<String, OrgConfig>,
}

/// Normalizes aliases in one file so the existing file-order merge remains authoritative.
pub(super) fn normalize_config_file(value: serde_yaml::Value) -> Result<serde_yaml::Value, String> {
    let serde_yaml::Value::Mapping(mut config) = value else {
        return Ok(value);
    };
    let wm = config.remove(serde_yaml::Value::String("wm".to_string()));
    let cc = config.remove(serde_yaml::Value::String("cc".to_string()));
    if wm.is_none() && cc.is_none() {
        return Ok(serde_yaml::Value::Mapping(config));
    }

    let legacy_agent = legacy_agent_from_sections([("wm", wm), ("cc", cc)])?;
    let agent_key = serde_yaml::Value::String("agent".to_string());
    let agent = match config.remove(&agent_key) {
        Some(agent) => merge_legacy_yaml(serde_yaml::Value::Mapping(legacy_agent), agent),
        None => serde_yaml::Value::Mapping(legacy_agent),
    };
    config.insert(agent_key, agent);

    Ok(serde_yaml::Value::Mapping(config))
}

/// Maps a legacy dot or environment path to its current path using the shared alias table.
pub(super) fn normalize_path(path: &str, separator: &str) -> Option<String> {
    let mut segments = path.split(separator);
    let section_name = segments.next()?;
    let field_name = segments.next()?;
    let section = legacy_section(section_name)?;
    let field = section
        .fields
        .iter()
        .find(|field| field.name == field_name)?;

    let mut current_segments: Vec<&str> = section.current_prefix.split('.').collect();
    current_segments.push(field.current_name);
    current_segments.extend(segments);
    Some(current_segments.join(separator))
}

pub(super) fn is_legacy_section_path(path: &str, separator: &str) -> bool {
    let Some(section_name) = path.split(separator).next() else {
        return false;
    };
    legacy_section(section_name).is_some()
}

pub(super) fn legacy_section_value(
    section_name: &str,
    agent: &serde_json::Value,
) -> Option<serde_json::Value> {
    let section = legacy_section(section_name)?;
    let mut object = serde_json::Map::new();
    let prefix = section
        .current_prefix
        .strip_prefix("agent")?
        .trim_start_matches('.');

    for field in section.fields {
        let path = if prefix.is_empty() {
            field.current_name.to_string()
        } else {
            format!("{prefix}.{}", field.current_name)
        };
        if let Some(value) = json_value_at_path(agent, &path) {
            object.insert(field.name.to_string(), value.clone());
        }
    }

    Some(serde_json::Value::Object(object))
}

fn json_value_at_path<'a>(
    value: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    path.split('.')
        .try_fold(value, |current, segment| current.as_object()?.get(segment))
}

fn legacy_agent_from_sections(
    sections: [(&str, Option<serde_yaml::Value>); 2],
) -> Result<serde_yaml::Mapping, String> {
    let mut agent = serde_yaml::Mapping::new();
    for (name, value) in sections {
        let Some(value) = value else {
            continue;
        };
        let section =
            legacy_section(name).ok_or_else(|| format!("unknown legacy section `{name}`"))?;
        let mapping = match value {
            serde_yaml::Value::Null => serde_yaml::Mapping::new(),
            serde_yaml::Value::Mapping(mapping) => mapping,
            _ => return Err(format!("`{name}` must be a mapping")),
        };

        for (key, value) in mapping {
            let serde_yaml::Value::String(name) = key else {
                return Err(format!("`{}` keys must be strings", section.name));
            };
            let Some(field) = section.fields.iter().find(|field| field.name == name) else {
                let expected = section
                    .fields
                    .iter()
                    .map(|field| format!("`{}`", field.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "unknown field `{name}`, expected one of {expected}"
                ));
            };

            let path = format!("{}.{}", section.current_prefix, field.current_name);
            let path = path.strip_prefix("agent.").unwrap_or(&path);
            insert_yaml_path(&mut agent, path, value);
        }
    }
    Ok(agent)
}

fn insert_yaml_path(root: &mut serde_yaml::Mapping, path: &str, value: serde_yaml::Value) {
    let mut segments = path.split('.');
    let Some(segment) = segments.next() else {
        return;
    };
    let Some(next_segment) = segments.next() else {
        root.insert(serde_yaml::Value::String(segment.to_string()), value);
        return;
    };

    let key = serde_yaml::Value::String(segment.to_string());
    let entry = root
        .entry(key)
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    if !matches!(entry, serde_yaml::Value::Mapping(_)) {
        *entry = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }
    if let serde_yaml::Value::Mapping(mapping) = entry {
        let remaining = std::iter::once(next_segment)
            .chain(segments)
            .collect::<Vec<_>>()
            .join(".");
        insert_yaml_path(mapping, &remaining, value);
    }
}

fn merge_legacy_yaml(base: serde_yaml::Value, overlay: serde_yaml::Value) -> serde_yaml::Value {
    match (base, overlay) {
        (serde_yaml::Value::Mapping(mut base), serde_yaml::Value::Mapping(overlay)) => {
            for (key, value) in overlay {
                let merged = match base.remove(&key) {
                    Some(existing) => merge_legacy_yaml(existing, value),
                    None => value,
                };
                base.insert(key, merged);
            }
            serde_yaml::Value::Mapping(base)
        }
        (base, serde_yaml::Value::Null) => base,
        (_, overlay) => overlay,
    }
}

fn legacy_section(name: &str) -> Option<&'static LegacySection> {
    LEGACY_SECTIONS.iter().find(|section| section.name == name)
}
