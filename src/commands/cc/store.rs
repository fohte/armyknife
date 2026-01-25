use std::fs;
use std::path::PathBuf;

use anyhow::Result;

use super::error::CcError;
use super::tty;
use super::types::Session;
use crate::shared::cache;

/// Returns the directory for storing Claude Code session data.
/// Path: ~/.cache/armyknife/cc/sessions/
pub fn sessions_dir() -> Result<PathBuf> {
    cache::base_dir()
        .map(|d| d.join("cc").join("sessions"))
        .ok_or_else(|| CcError::CacheDirNotFound.into())
}

/// Returns the file path for a specific session.
/// Path: ~/.cache/armyknife/cc/sessions/<session_id>.json
///
/// Validates that session_id does not contain path separators to prevent
/// path traversal attacks.
pub fn session_file(session_id: &str) -> Result<PathBuf> {
    // Reject session IDs with path separators to prevent path traversal
    if session_id.contains('/') || session_id.contains('\\') || session_id.contains("..") {
        return Err(CcError::InvalidSessionId(session_id.to_string()).into());
    }

    sessions_dir().map(|d| d.join(format!("{session_id}.json")))
}

/// Loads a session from disk by session ID.
/// Returns Ok(None) if the session file doesn't exist.
pub fn load_session(session_id: &str) -> Result<Option<Session>> {
    let path = session_file(session_id)?;

    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path)?;
    let session: Session = serde_json::from_str(&content)?;
    Ok(Some(session))
}

/// Saves a session to disk.
/// Creates the parent directory if it doesn't exist.
pub fn save_session(session: &Session) -> Result<()> {
    let path = session_file(&session.session_id)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let content = serde_json::to_string_pretty(session)?;
    fs::write(&path, content)?;
    Ok(())
}

/// Deletes a session from disk.
/// Returns Ok(()) even if the session file doesn't exist.
pub fn delete_session(session_id: &str) -> Result<()> {
    let path = session_file(session_id)?;

    if path.exists() {
        fs::remove_file(&path)?;
    }

    Ok(())
}

/// Lists all sessions from disk.
/// Reads all .json files in the sessions directory.
pub fn list_sessions() -> Result<Vec<Session>> {
    let dir = sessions_dir()?;

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "json")
            && let Ok(content) = fs::read_to_string(&path)
            && let Ok(session) = serde_json::from_str::<Session>(&content)
        {
            sessions.push(session);
        }
    }

    // Sort by updated_at descending (most recent first)
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(sessions)
}

/// Removes sessions whose TTY no longer exists.
/// This cleans up stale sessions from terminated terminals.
pub fn cleanup_stale_sessions() -> Result<()> {
    let dir = sessions_dir()?;

    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "json")
            && let Ok(content) = fs::read_to_string(&path)
            && let Ok(session) = serde_json::from_str::<Session>(&content)
            && let Some(ref tty_path) = session.tty
            && !tty::is_tty_alive(tty_path)
        {
            // Remove if TTY exists but is no longer valid
            let _ = fs::remove_file(&path);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::SessionStatus;
    use chrono::Utc;
    use tempfile::TempDir;

    fn create_test_session(id: &str) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: Some("/dev/ttys001".to_string()),
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
        }
    }

    #[test]
    fn test_session_serialization() {
        let session = create_test_session("test-123");
        let json = serde_json::to_string(&session).expect("serialization should succeed");
        let parsed: Session = serde_json::from_str(&json).expect("deserialization should succeed");

        assert_eq!(parsed.session_id, "test-123");
        assert_eq!(parsed.status, SessionStatus::Running);
    }

    #[test]
    fn test_save_and_load_session() {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let sessions_path = temp_dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_path).expect("dir creation should succeed");

        let session = create_test_session("save-load-test");
        let file_path = sessions_path.join("save-load-test.json");

        // Save session
        let content = serde_json::to_string_pretty(&session).expect("serialization should succeed");
        std::fs::write(&file_path, content).expect("write should succeed");

        // Load session
        let loaded_content = std::fs::read_to_string(&file_path).expect("read should succeed");
        let loaded: Session =
            serde_json::from_str(&loaded_content).expect("deserialization should succeed");

        assert_eq!(loaded.session_id, "save-load-test");
    }

    #[test]
    fn test_session_file_rejects_path_traversal() {
        // Should reject session IDs with path separators
        assert!(session_file("../etc/passwd").is_err());
        assert!(session_file("foo/bar").is_err());
        assert!(session_file("foo\\bar").is_err());
        assert!(session_file("..").is_err());

        // Should accept valid session IDs
        assert!(session_file("valid-session-id").is_ok());
        assert!(session_file("session_123").is_ok());
    }
}
