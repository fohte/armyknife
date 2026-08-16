//! Reader for Claude Code's own per-process session registry
//! (`~/.claude/sessions/<pid>.json`).
//!
//! This file is written by the Claude Code CLI itself, not by armyknife. Its
//! `name` field is the only identifier `SendMessage`/`ListAgents` accept as a
//! target, and armyknife has no other source for it (`ListAgents` output is
//! not addressable programmatically). This module is the single place that
//! understands the file's shape, so a future Claude Code format change only
//! needs a fix here.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RegistryEntry {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(default)]
    name: Option<String>,
}

/// Reads every `~/.claude/sessions/*.json` file and returns a
/// `session_id -> name` map. Entries with no name, and files that don't
/// parse (unrelated `.lock`/`.key` companions, a corrupted or
/// future/incompatible format), are silently skipped rather than failing the
/// whole lookup -- callers treat a missing entry as "name unresolvable".
pub fn load_name_map() -> HashMap<String, String> {
    let Some(home) = crate::shared::dirs::home_dir() else {
        return HashMap::new();
    };
    load_name_map_in(&home)
}

fn load_name_map_in(home: &Path) -> HashMap<String, String> {
    let dir = home.join(".claude").join("sessions");

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return HashMap::new();
    };

    let mut map = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();

        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(&path)
            && let Ok(entry) = serde_json::from_str::<RegistryEntry>(&content)
            && let Some(name) = entry.name.filter(|n| !n.is_empty())
        {
            map.insert(entry.session_id, name);
        }
    }
    map
}

/// Resolves a single Claude Code session ID to its `SendMessage` target name.
pub fn resolve_name(session_id: &str) -> Option<String> {
    load_name_map().remove(session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_registry_file(home: &Path, pid: &str, content: &str) {
        let dir = home.join(".claude").join("sessions");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{pid}.json")), content).unwrap();
    }

    #[test]
    fn load_name_map_in_joins_by_session_id() {
        let home = TempDir::new().unwrap();
        write_registry_file(
            home.path(),
            "111",
            r#"{"pid":111,"sessionId":"aaa","name":"repo-1a"}"#,
        );
        write_registry_file(
            home.path(),
            "222",
            r#"{"pid":222,"sessionId":"bbb","name":"repo-2b"}"#,
        );

        let map = load_name_map_in(home.path());

        assert_eq!(
            map,
            HashMap::from([
                ("aaa".to_string(), "repo-1a".to_string()),
                ("bbb".to_string(), "repo-2b".to_string()),
            ])
        );
    }

    #[test]
    fn load_name_map_in_skips_entries_without_a_name() {
        let home = TempDir::new().unwrap();
        write_registry_file(home.path(), "111", r#"{"pid":111,"sessionId":"aaa"}"#);

        let map = load_name_map_in(home.path());

        assert_eq!(map, HashMap::new());
    }

    #[test]
    fn load_name_map_in_skips_non_json_companion_files() {
        let home = TempDir::new().unwrap();
        let dir = home.path().join(".claude").join("sessions");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("111.key"), "not json").unwrap();
        fs::write(dir.join("111.json.lock"), "").unwrap();
        write_registry_file(
            home.path(),
            "111",
            r#"{"pid":111,"sessionId":"aaa","name":"repo-1a"}"#,
        );

        let map = load_name_map_in(home.path());

        assert_eq!(
            map,
            HashMap::from([("aaa".to_string(), "repo-1a".to_string())])
        );
    }

    #[test]
    fn load_name_map_in_returns_empty_when_sessions_dir_is_missing() {
        let home = TempDir::new().unwrap();

        let map = load_name_map_in(home.path());

        assert_eq!(map, HashMap::new());
    }
}
