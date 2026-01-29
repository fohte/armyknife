use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;

use super::error::CcError;
use super::tty;
use super::types::Session;
use crate::shared::cache;

/// Lock timeout for file operations.
/// Short timeout to minimize performance impact while preventing race conditions.
const LOCK_TIMEOUT: Duration = Duration::from_millis(500);

/// Number of retry attempts for acquiring a lock.
const LOCK_RETRY_COUNT: u32 = 10;

/// Delay between lock retry attempts.
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);

/// Returns the directory for storing Claude Code session data.
/// Path: ~/Library/Caches/armyknife/cc/sessions/ (macOS) or ~/.cache/armyknife/cc/sessions/ (Linux)
pub fn sessions_dir() -> Result<PathBuf> {
    cache::base_dir()
        .map(|d| d.join("cc").join("sessions"))
        .ok_or_else(|| CcError::CacheDirNotFound.into())
}

/// Returns the file path for a specific session.
/// Path: ~/Library/Caches/armyknife/cc/sessions/<session_id>.json (macOS)
///       ~/.cache/armyknife/cc/sessions/<session_id>.json (Linux)
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

/// Acquires an exclusive lock on a file with timeout and retry.
/// Uses try_lock() which acquires an exclusive (write) lock.
fn acquire_lock(file: &File) -> Result<()> {
    for attempt in 0..LOCK_RETRY_COUNT {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) => {
                // Lock is held by another process, retry after delay
                if attempt < LOCK_RETRY_COUNT - 1 {
                    std::thread::sleep(LOCK_RETRY_DELAY);
                }
            }
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
    }

    // All retries exhausted
    Err(CcError::LockTimeout(LOCK_TIMEOUT).into())
}

/// Loads a session from disk by session ID.
/// Returns Ok(None) if the session file doesn't exist.
///
/// If the session file is corrupted (invalid JSON), it will be deleted
/// and Ok(None) will be returned to allow recovery.
pub fn load_session(session_id: &str) -> Result<Option<Session>> {
    let path = session_file(session_id)?;

    if !path.exists() {
        return Ok(None);
    }

    // Open file with shared lock for reading
    let file = match File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    // Acquire shared lock for reading
    acquire_shared_lock(&file)?;

    // Read content while holding the lock
    let mut content = String::new();
    let mut reader = std::io::BufReader::new(&file);
    reader.read_to_string(&mut content)?;

    // Lock is automatically released when file is dropped
    drop(file);

    // Parse JSON
    match serde_json::from_str::<Session>(&content) {
        Ok(session) => Ok(Some(session)),
        Err(_) => {
            // File is corrupted, delete it and return None for recovery
            eprintln!(
                "[armyknife] warning: session file corrupted, deleting: {}",
                path.display()
            );
            let _ = fs::remove_file(&path);
            Ok(None)
        }
    }
}

/// Acquires a shared lock on a file with timeout and retry.
fn acquire_shared_lock(file: &File) -> Result<()> {
    for attempt in 0..LOCK_RETRY_COUNT {
        match file.try_lock_shared() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) => {
                if attempt < LOCK_RETRY_COUNT - 1 {
                    std::thread::sleep(LOCK_RETRY_DELAY);
                }
            }
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
    }
    Err(CcError::LockTimeout(LOCK_TIMEOUT).into())
}

/// Saves a session to disk with exclusive file locking.
/// Creates the parent directory if it doesn't exist.
///
/// Uses atomic write (write to temp file, then rename) combined with
/// file locking to prevent race conditions when multiple hooks run concurrently.
pub fn save_session(session: &Session) -> Result<()> {
    let path = session_file(&session.session_id)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Create lock file path (use .lock suffix to avoid conflicts)
    let lock_path = path.with_extension("json.lock");

    // Open or create lock file
    let lock_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;

    // Acquire exclusive lock
    acquire_lock(&lock_file)?;

    // Serialize content
    let content = serde_json::to_string_pretty(session)?;

    // Write to temporary file first for atomic operation
    let temp_path = path.with_extension("json.tmp");
    let mut temp_file = File::create(&temp_path)?;
    temp_file.write_all(content.as_bytes())?;
    temp_file.sync_all()?;

    // Rename temporary file to target (atomic on Unix)
    fs::rename(&temp_path, &path)?;

    // Lock is automatically released when lock_file is dropped
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
            current_tool: None,
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

    mod file_lock_tests {
        use super::*;
        use rstest::{fixture, rstest};
        use std::sync::{Arc, Barrier};
        use std::thread;

        struct TempSessionDir {
            #[expect(dead_code, reason = "kept alive to prevent cleanup until dropped")]
            temp_dir: TempDir,
            sessions_path: PathBuf,
        }

        #[fixture]
        fn temp_session_dir() -> TempSessionDir {
            let temp_dir = TempDir::new().expect("temp dir creation should succeed");
            let sessions_path = temp_dir.path().join("sessions");
            fs::create_dir_all(&sessions_path).expect("dir creation should succeed");
            TempSessionDir {
                temp_dir,
                sessions_path,
            }
        }

        #[rstest]
        fn acquire_lock_succeeds_on_unlocked_file(temp_session_dir: TempSessionDir) {
            let lock_path = temp_session_dir.sessions_path.join("test.lock");

            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
                .expect("file creation should succeed");

            let result = acquire_lock(&file);
            assert!(result.is_ok(), "should acquire lock on unlocked file");
        }

        #[rstest]
        fn acquire_shared_lock_succeeds_on_unlocked_file(temp_session_dir: TempSessionDir) {
            let test_file = temp_session_dir.sessions_path.join("test.json");

            fs::write(&test_file, "{}").expect("write should succeed");

            let file = File::open(&test_file).expect("open should succeed");
            let result = acquire_shared_lock(&file);
            assert!(
                result.is_ok(),
                "should acquire shared lock on unlocked file"
            );
        }

        #[rstest]
        fn multiple_shared_locks_allowed(temp_session_dir: TempSessionDir) {
            let test_file = temp_session_dir.sessions_path.join("test.json");

            fs::write(&test_file, "{}").expect("write should succeed");

            let file1 = File::open(&test_file).expect("open should succeed");
            let file2 = File::open(&test_file).expect("open should succeed");

            let result1 = acquire_shared_lock(&file1);
            let result2 = acquire_shared_lock(&file2);

            assert!(result1.is_ok(), "first shared lock should succeed");
            assert!(result2.is_ok(), "second shared lock should succeed");
        }

        #[rstest]
        fn concurrent_save_does_not_corrupt_file() {
            let num_threads = 10;
            let barrier = Arc::new(Barrier::new(num_threads));

            let cache_sessions_dir = sessions_dir().expect("sessions_dir should succeed");
            fs::create_dir_all(&cache_sessions_dir).expect("cache dir creation should succeed");

            let session_id = format!("concurrent-test-{}", std::process::id());
            let handles: Vec<_> = (0..num_threads)
                .map(|i| {
                    let barrier = Arc::clone(&barrier);
                    let session_id = session_id.clone();
                    thread::spawn(move || {
                        let mut session = create_test_session(&session_id);
                        session.last_message = Some(format!("Message from thread {}", i));

                        barrier.wait();
                        save_session(&session)
                    })
                })
                .collect();

            let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

            for (i, result) in results.iter().enumerate() {
                assert!(
                    result.is_ok(),
                    "thread {} save should succeed: {:?}",
                    i,
                    result
                );
            }

            let loaded = load_session(&session_id).expect("load should succeed");
            assert!(loaded.is_some(), "session should be loadable");
            assert_eq!(loaded.unwrap().session_id, session_id);

            // Cleanup
            let _ = delete_session(&session_id);
            let lock_path = session_file(&session_id)
                .expect("session_file should succeed")
                .with_extension("json.lock");
            let _ = fs::remove_file(&lock_path);
        }
    }

    mod corrupted_file_recovery_tests {
        use super::*;
        use rstest::rstest;

        /// Helper to create a unique session ID and ensure cache dir exists.
        fn setup_session(prefix: &str) -> (String, PathBuf) {
            let cache_sessions_dir = sessions_dir().expect("sessions_dir should succeed");
            fs::create_dir_all(&cache_sessions_dir).expect("cache dir creation should succeed");

            let session_id = format!("{}-{}", prefix, std::process::id());
            let path = session_file(&session_id).expect("session_file should succeed");
            (session_id, path)
        }

        #[rstest]
        #[case::corrupted_with_extra_data(
            r#"{"session_id": "test", "current_tool": "Bash(cd ...)"}>/dev/null)"}"#,
            "corrupted"
        )]
        #[case::truncated_json(r#"{"session_id": "test", "cwd": "/tmp""#, "truncated")]
        #[case::empty_file("", "empty")]
        fn load_invalid_json_returns_none_and_deletes_file(
            #[case] content: &str,
            #[case] prefix: &str,
        ) {
            let (session_id, path) = setup_session(prefix);

            fs::write(&path, content).expect("write should succeed");

            let result = load_session(&session_id).expect("load should not error");
            assert!(result.is_none(), "{} session should return None", prefix);
            assert!(!path.exists(), "{} file should be deleted", prefix);
        }

        #[rstest]
        fn load_valid_session_succeeds() {
            let (session_id, _path) = setup_session("valid");
            let session = create_test_session(&session_id);

            save_session(&session).expect("save should succeed");

            let loaded = load_session(&session_id).expect("load should succeed");
            assert!(loaded.is_some(), "valid session should load");
            assert_eq!(loaded.unwrap().session_id, session_id);

            // Cleanup
            let _ = delete_session(&session_id);
            let lock_path = session_file(&session_id)
                .expect("session_file should succeed")
                .with_extension("json.lock");
            let _ = fs::remove_file(&lock_path);
        }
    }
}
