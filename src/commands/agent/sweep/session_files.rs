use std::fs;
use std::path::Path;

use crate::commands::agent::types::Session;
use anyhow::{Context, Result};

pub(super) fn session_ids_in(sessions_dir: &Path) -> Result<Vec<String>> {
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(sessions_dir)
        .with_context(|| format!("reading sessions dir {}", sessions_dir.display()))?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();

    Ok(entries
        .into_iter()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                return None;
            }
            let is_archived = fs::read_to_string(&path)
                .ok()
                .and_then(|content| serde_json::from_str::<Session>(&content).ok())
                .is_some_and(|session| session.archived_at.is_some());
            if is_archived {
                return None;
            }
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::session_ids_in;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn write_session(directory: &std::path::Path, session_id: &str, archived: bool) {
        let mut value = json!({
            "session_id": session_id,
            "cwd": "/tmp/example-worktree",
            "transcript_path": null,
            "tmux_info": null,
            "status": "stopped",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        });
        if archived {
            value["archived_at"] = json!("2026-01-01T00:00:00Z");
        }
        fs::write(
            directory.join(format!("{session_id}.json")),
            serde_json::to_vec(&value).expect("session JSON should serialize"),
        )
        .expect("session file should write");
    }

    #[test]
    fn excludes_only_parsed_archived_sessions_from_sweep_scans() {
        let temp_dir = TempDir::new().expect("temp directory creation should succeed");
        write_session(temp_dir.path(), "active-session", false);
        write_session(temp_dir.path(), "archived-session", true);
        fs::write(temp_dir.path().join("malformed-session.json"), "not json")
            .expect("malformed session file should write");

        let mut actual = session_ids_in(temp_dir.path()).expect("session scan should succeed");
        actual.sort();
        assert_eq!(
            actual,
            vec![
                "active-session".to_string(),
                "malformed-session".to_string(),
            ]
        );
    }
}
