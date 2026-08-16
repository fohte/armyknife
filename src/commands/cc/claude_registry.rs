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
    /// Unix-epoch milliseconds the process started. Used to break ties when
    /// the same `session_id` shows up in more than one file -- e.g. a
    /// crashed process's file lingering alongside the file of a later
    /// `claude --resume` of the same session -- by keeping the name from
    /// the most recently started process instead of whichever `read_dir`
    /// happens to list last.
    #[serde(default, rename = "startedAt")]
    started_at: Option<u64>,
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

    let mut best: HashMap<String, (String, Option<u64>)> = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();

        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<RegistryEntry>(&content) else {
            continue;
        };
        let Some(name) = parsed.name.filter(|n| !n.is_empty()) else {
            continue;
        };

        let is_older = best
            .get(&parsed.session_id)
            .is_some_and(|(_, existing_started_at)| *existing_started_at >= parsed.started_at);
        if !is_older {
            best.insert(parsed.session_id, (name, parsed.started_at));
        }
    }

    best.into_iter()
        .map(|(session_id, (name, _))| (session_id, name))
        .collect()
}

/// Resolves a single Claude Code session ID to its `SendMessage` target name.
pub fn resolve_name(session_id: &str) -> Option<String> {
    load_name_map().remove(session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs;
    use tempfile::TempDir;

    fn write_files(home: &Path, files: &[(&str, &str)]) {
        let dir = home.join(".claude").join("sessions");
        fs::create_dir_all(&dir).unwrap();
        for (filename, content) in files {
            fs::write(dir.join(filename), content).unwrap();
        }
    }

    fn expected_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[rstest]
    #[case::joins_multiple_entries_by_session_id(
        &[
            ("111.json", r#"{"pid":111,"sessionId":"aaa","name":"repo-1a"}"#),
            ("222.json", r#"{"pid":222,"sessionId":"bbb","name":"repo-2b"}"#),
        ],
        &[("aaa", "repo-1a"), ("bbb", "repo-2b")]
    )]
    #[case::skips_entries_without_a_name(
        &[("111.json", r#"{"pid":111,"sessionId":"aaa"}"#)],
        &[]
    )]
    #[case::skips_non_json_companion_files(
        &[
            ("111.key", "not json"),
            ("111.json.lock", ""),
            ("111.json", r#"{"pid":111,"sessionId":"aaa","name":"repo-1a"}"#),
        ],
        &[("aaa", "repo-1a")]
    )]
    #[case::prefers_the_entry_with_the_later_started_at_on_session_id_collision(
        &[
            ("111.json", r#"{"pid":111,"sessionId":"aaa","name":"stale-crashed","startedAt":1000}"#),
            ("222.json", r#"{"pid":222,"sessionId":"aaa","name":"resumed","startedAt":2000}"#),
        ],
        &[("aaa", "resumed")]
    )]
    #[case::prefers_a_timestamped_entry_over_one_with_no_started_at(
        &[
            ("111.json", r#"{"pid":111,"sessionId":"aaa","name":"no-timestamp"}"#),
            ("222.json", r#"{"pid":222,"sessionId":"aaa","name":"timestamped","startedAt":2000}"#),
        ],
        &[("aaa", "timestamped")]
    )]
    fn load_name_map_in_cases(#[case] files: &[(&str, &str)], #[case] expected: &[(&str, &str)]) {
        let home = TempDir::new().unwrap();
        write_files(home.path(), files);

        let map = load_name_map_in(home.path());

        assert_eq!(map, expected_map(expected));
    }

    #[test]
    fn load_name_map_in_returns_empty_when_sessions_dir_is_missing() {
        let home = TempDir::new().unwrap();

        let map = load_name_map_in(home.path());

        assert_eq!(map, HashMap::new());
    }
}
