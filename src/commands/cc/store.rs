use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;

use super::error::CcError;
use super::types::Session;
use crate::infra::tmux;
use crate::shared::cache;

/// Threshold in seconds for sort stability.
/// Sessions updated within this window are sorted by created_at instead,
/// preventing rapid reordering during concurrent agent execution.
const SORT_STABILITY_THRESHOLD_SECS: i64 = 30;

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

/// Returns the file path for a specific session within a given directory.
/// This is the internal implementation that accepts a custom sessions directory.
fn session_file_in(sessions_dir: &Path, session_id: &str) -> Result<PathBuf> {
    // Reject session IDs with path separators to prevent path traversal
    if session_id.contains('/') || session_id.contains('\\') || session_id.contains("..") {
        return Err(CcError::InvalidSessionId(session_id.to_string()).into());
    }

    Ok(sessions_dir.join(format!("{session_id}.json")))
}

/// Returns the file path for a specific session.
/// Path: ~/Library/Caches/armyknife/cc/sessions/<session_id>.json (macOS)
///       ~/.cache/armyknife/cc/sessions/<session_id>.json (Linux)
///
/// Validates that session_id does not contain path separators to prevent
/// path traversal attacks.
#[cfg(test)]
pub fn session_file(session_id: &str) -> Result<PathBuf> {
    session_file_in(&sessions_dir()?, session_id)
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
/// If the session file is corrupted (invalid JSON), Ok(None) will be returned.
/// The corrupted file is not deleted here to avoid race conditions; instead,
/// the next save_session call will atomically overwrite it.
pub fn load_session(session_id: &str) -> Result<Option<Session>> {
    let dir = sessions_dir()?;
    load_session_from(&dir, session_id)
}

/// Loads a session from a specific directory.
/// Allows testing with temporary directories.
pub(crate) fn load_session_from(sessions_dir: &Path, session_id: &str) -> Result<Option<Session>> {
    let path = session_file_in(sessions_dir, session_id)?;

    if !path.exists() {
        return Ok(None);
    }

    // Use the same lock file as save_session to coordinate readers and writers
    let lock_path = path.with_extension("json.lock");

    // Open or create lock file for shared lock
    let lock_file = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    // Acquire shared lock (allows concurrent reads, blocks exclusive writes)
    acquire_shared_lock(&lock_file)?;

    // Read the actual session file while holding the lock
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    // Parse JSON (lock is held until lock_file is dropped)
    match serde_json::from_str::<Session>(&content) {
        Ok(session) => Ok(Some(session)),
        Err(_) => {
            // File is corrupted; return None and let save_session overwrite it
            eprintln!(
                "[armyknife] warning: session file corrupted: {}",
                path.display()
            );
            Ok(None)
        }
    }
    // Lock is automatically released when lock_file is dropped
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

/// Saves a session to a specific directory.
/// Allows testing with temporary directories.
pub(crate) fn save_session_to(sessions_dir: &Path, session: &Session) -> Result<()> {
    let path = session_file_in(sessions_dir, &session.session_id)?;

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
    delete_session_from(&sessions_dir()?, session_id)
}

/// Deletes a session from a specific directory.
/// Allows testing with temporary directories.
pub(crate) fn delete_session_from(sessions_dir: &Path, session_id: &str) -> Result<()> {
    let path = session_file_in(sessions_dir, session_id)?;

    if path.exists() {
        fs::remove_file(&path)?;
    }

    Ok(())
}

/// Sorts sessions by updated_at descending with stability threshold.
///
/// Sessions updated within [`SORT_STABILITY_THRESHOLD_SECS`] of each other
/// are considered equivalent and sorted by created_at descending instead.
/// This prevents rapid reordering when multiple sessions are updated concurrently.
pub fn sort_sessions(sessions: &mut [Session]) {
    sessions.sort_by(|a, b| {
        let bucket_a = a.updated_at.timestamp() / SORT_STABILITY_THRESHOLD_SECS;
        let bucket_b = b.updated_at.timestamp() / SORT_STABILITY_THRESHOLD_SECS;
        bucket_b
            .cmp(&bucket_a)
            .then_with(|| b.created_at.cmp(&a.created_at))
    });
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

    sort_sessions(&mut sessions);

    Ok(sessions)
}

/// Removes stale sessions from disk.
///
/// A session is considered stale and removed if its tmux pane no longer exists.
/// Sessions without tmux_info are kept (they may be running outside tmux).
/// If tmux server is not available, cleanup is skipped to avoid incorrectly
/// deleting sessions (is_pane_alive returns false when server is down).
pub fn cleanup_stale_sessions() -> Result<()> {
    // Skip cleanup if tmux server is not available to avoid false negatives
    if !tmux::is_server_available() {
        return Ok(());
    }

    cleanup_stale_sessions_impl(tmux::is_pane_alive)
}

fn cleanup_stale_sessions_impl<F>(is_pane_alive: F) -> Result<()>
where
    F: Fn(&str) -> bool,
{
    let dir = sessions_dir()?;

    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }

        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };

        let Ok(session) = serde_json::from_str::<Session>(&content) else {
            continue;
        };

        let should_remove = session
            .tmux_info
            .as_ref()
            .is_some_and(|info| !is_pane_alive(&info.pane_id));

        if should_remove {
            let _ = fs::remove_file(&path);
            // Also clean up the lock file if it exists
            let lock_path = path.with_extension("json.lock");
            let _ = fs::remove_file(&lock_path);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::SessionStatus;
    use chrono::Utc;
    use rstest::fixture;
    use tempfile::TempDir;

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

    fn create_test_session(id: &str) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
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
        use rstest::rstest;
        use std::sync::{Arc, Barrier};
        use std::thread;

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
        fn concurrent_save_does_not_corrupt_file(temp_session_dir: TempSessionDir) {
            let num_threads = 10;
            let barrier = Arc::new(Barrier::new(num_threads));
            let sessions_path = Arc::new(temp_session_dir.sessions_path.clone());

            let session_id = format!("concurrent-test-{}", std::process::id());
            let handles: Vec<_> = (0..num_threads)
                .map(|i| {
                    let barrier = Arc::clone(&barrier);
                    let session_id = session_id.clone();
                    let sessions_path = Arc::clone(&sessions_path);
                    thread::spawn(move || {
                        let mut session = create_test_session(&session_id);
                        session.last_message = Some(format!("Message from thread {}", i));

                        barrier.wait();
                        save_session_to(&sessions_path, &session)
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

            let loaded = load_session_from(&temp_session_dir.sessions_path, &session_id)
                .expect("load should succeed");
            assert!(loaded.is_some(), "session should be loadable");
            assert_eq!(loaded.unwrap().session_id, session_id);
        }
    }

    mod cleanup_stale_sessions_tests {
        use super::*;
        use crate::commands::cc::types::TmuxInfo;
        use rstest::rstest;

        /// Mock that treats all panes as dead
        fn mock_pane_always_dead(_pane_id: &str) -> bool {
            false
        }

        /// Mock that treats all panes as alive
        fn mock_pane_always_alive(_pane_id: &str) -> bool {
            true
        }

        /// Test helper that processes only a single session file.
        /// This ensures test isolation without affecting other parallel tests.
        fn cleanup_single_session_in<F>(
            sessions_dir: &Path,
            session_id: &str,
            is_pane_alive: F,
        ) -> Result<()>
        where
            F: Fn(&str) -> bool,
        {
            let path = session_file_in(sessions_dir, session_id)?;

            if !path.exists() {
                return Ok(());
            }

            let content = fs::read_to_string(&path)?;
            let session: Session = serde_json::from_str(&content)?;

            let should_remove = session
                .tmux_info
                .as_ref()
                .is_some_and(|info| !is_pane_alive(&info.pane_id));

            if should_remove {
                let _ = fs::remove_file(&path);
                let lock_path = path.with_extension("json.lock");
                let _ = fs::remove_file(&lock_path);
            }

            Ok(())
        }

        #[rstest]
        fn removes_session_with_nonexistent_pane(temp_session_dir: TempSessionDir) {
            let session_id = "dead-pane-test";
            let path = session_file_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_file_in should succeed");

            let mut session = create_test_session(session_id);
            session.tmux_info = Some(TmuxInfo {
                session_name: "test".to_string(),
                window_name: "test".to_string(),
                window_index: 0,
                pane_id: "%99999".to_string(),
            });
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");
            assert!(path.exists(), "session file should exist before cleanup");

            cleanup_single_session_in(
                &temp_session_dir.sessions_path,
                session_id,
                mock_pane_always_dead,
            )
            .expect("cleanup should succeed");

            assert!(
                !path.exists(),
                "session with nonexistent pane should be removed"
            );
        }

        #[rstest]
        fn keeps_session_with_alive_pane(temp_session_dir: TempSessionDir) {
            let session_id = "alive-pane-test";
            let path = session_file_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_file_in should succeed");

            let mut session = create_test_session(session_id);
            session.tmux_info = Some(TmuxInfo {
                session_name: "test".to_string(),
                window_name: "test".to_string(),
                window_index: 0,
                pane_id: "%1".to_string(),
            });
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");

            cleanup_single_session_in(
                &temp_session_dir.sessions_path,
                session_id,
                mock_pane_always_alive,
            )
            .expect("cleanup should succeed");

            assert!(path.exists(), "session with alive pane should be kept");
        }

        #[rstest]
        fn keeps_session_without_tmux_info(temp_session_dir: TempSessionDir) {
            let session_id = "no-tmux-test";
            let path = session_file_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_file_in should succeed");

            let mut session = create_test_session(session_id);
            session.tmux_info = None;
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");

            // Even with mock that treats all panes as dead, sessions without
            // tmux_info should be kept.
            cleanup_single_session_in(
                &temp_session_dir.sessions_path,
                session_id,
                mock_pane_always_dead,
            )
            .expect("cleanup should succeed");

            assert!(path.exists(), "session without tmux_info should be kept");
        }

        #[rstest]
        fn also_removes_lock_file(temp_session_dir: TempSessionDir) {
            let session_id = "with-lock-test";
            let path = session_file_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_file_in should succeed");

            let mut session = create_test_session(session_id);
            session.tmux_info = Some(TmuxInfo {
                session_name: "test".to_string(),
                window_name: "test".to_string(),
                window_index: 0,
                pane_id: "%99999".to_string(),
            });
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");

            let lock_path = path.with_extension("json.lock");
            assert!(path.exists(), "session file should exist");

            cleanup_single_session_in(
                &temp_session_dir.sessions_path,
                session_id,
                mock_pane_always_dead,
            )
            .expect("cleanup should succeed");

            assert!(!path.exists(), "session file should be removed");
            assert!(!lock_path.exists(), "lock file should also be removed");
        }
    }

    mod corrupted_file_recovery_tests {
        use super::*;
        use rstest::rstest;

        #[rstest]
        #[case::corrupted_with_extra_data(
            r#"{"session_id": "test", "current_tool": "Bash(cd ...)"}>/dev/null)"}"#,
            "corrupted"
        )]
        #[case::truncated_json(r#"{"session_id": "test", "cwd": "/tmp""#, "truncated")]
        #[case::empty_file("", "empty")]
        fn load_invalid_json_returns_none(
            temp_session_dir: TempSessionDir,
            #[case] content: &str,
            #[case] prefix: &str,
        ) {
            let session_id = format!("{}-test", prefix);
            let path = session_file_in(&temp_session_dir.sessions_path, &session_id)
                .expect("session_file_in should succeed");

            fs::write(&path, content).expect("write should succeed");

            let result = load_session_from(&temp_session_dir.sessions_path, &session_id)
                .expect("load should not error");
            assert!(result.is_none(), "{} session should return None", prefix);
            // File is not deleted to avoid race conditions; save_session will overwrite it
            assert!(path.exists(), "{} file should still exist", prefix);
        }

        #[rstest]
        fn load_valid_session_succeeds(temp_session_dir: TempSessionDir) {
            let session_id = "valid-test";
            let session = create_test_session(session_id);

            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");

            let loaded = load_session_from(&temp_session_dir.sessions_path, session_id)
                .expect("load should succeed");
            assert!(loaded.is_some(), "valid session should load");
            assert_eq!(loaded.unwrap().session_id, session_id);
        }
    }

    mod sort_sessions_tests {
        use super::*;
        use chrono::{DateTime, TimeDelta};

        /// Fixed base time for deterministic tests.
        /// Aligned to a bucket boundary (timestamp divisible by 30) so that
        /// small positive deltas stay within the same bucket.
        fn base_time() -> DateTime<Utc> {
            DateTime::from_timestamp(1_700_000_400, 0).expect("valid fixed timestamp")
        }

        fn session_with_times(
            id: &str,
            created_at: DateTime<Utc>,
            updated_at: DateTime<Utc>,
        ) -> Session {
            Session {
                session_id: id.to_string(),
                cwd: PathBuf::from("/tmp/test"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Running,
                created_at,
                updated_at,
                last_message: None,
                current_tool: None,
            }
        }

        #[test]
        fn within_threshold_sorted_by_created_at() {
            let base = base_time();
            // Both updated_at values fit within the same 30s bucket
            let s1 = session_with_times(
                "older-created",
                base - TimeDelta::seconds(10),
                base + TimeDelta::seconds(5),
            );
            let s2 = session_with_times("newer-created", base, base);

            let mut sessions = vec![s1, s2];
            sort_sessions(&mut sessions);

            // newer created_at should come first (descending)
            assert_eq!(sessions[0].session_id, "newer-created");
            assert_eq!(sessions[1].session_id, "older-created");
        }

        #[test]
        fn beyond_threshold_sorted_by_updated_at() {
            let base = base_time();
            // Ensure sessions fall into different buckets by using a large gap
            let s1 = session_with_times(
                "old-update",
                base - TimeDelta::seconds(120),
                base - TimeDelta::seconds(60),
            );
            let s2 = session_with_times("new-update", base - TimeDelta::seconds(100), base);

            let mut sessions = vec![s1, s2];
            sort_sessions(&mut sessions);

            // More recently updated should come first
            assert_eq!(sessions[0].session_id, "new-update");
            assert_eq!(sessions[1].session_id, "old-update");
        }

        #[test]
        fn same_bucket_stable_by_created_at_descending() {
            let base = base_time();
            // All within the same 30s bucket
            let s1 = session_with_times("a", base - TimeDelta::seconds(20), base);
            let s2 = session_with_times(
                "b",
                base - TimeDelta::seconds(10),
                base + TimeDelta::seconds(3),
            );
            let s3 = session_with_times("c", base, base + TimeDelta::seconds(5));

            let mut sessions = vec![s1, s2, s3];
            sort_sessions(&mut sessions);

            // Should be sorted by created_at descending: c, b, a
            assert_eq!(sessions[0].session_id, "c");
            assert_eq!(sessions[1].session_id, "b");
            assert_eq!(sessions[2].session_id, "a");
        }

        #[test]
        fn mixed_buckets_and_tiebreaker() {
            let base = base_time();
            // s1 and s2 in the same bucket (recent), s3 in an older bucket
            let s1 = session_with_times("recent-old-created", base - TimeDelta::seconds(10), base);
            let s2 = session_with_times("recent-new-created", base, base + TimeDelta::seconds(2));
            let s3 = session_with_times(
                "old-bucket",
                base - TimeDelta::seconds(5),
                base - TimeDelta::seconds(60),
            );

            let mut sessions = vec![s3, s1, s2];
            sort_sessions(&mut sessions);

            // Recent bucket first (s2, s1 by created_at desc), then old bucket (s3)
            assert_eq!(sessions[0].session_id, "recent-new-created");
            assert_eq!(sessions[1].session_id, "recent-old-created");
            assert_eq!(sessions[2].session_id, "old-bucket");
        }
    }
}
