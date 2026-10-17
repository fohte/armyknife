use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::{cell::RefCell, collections::HashSet};

#[cfg(test)]
use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::error::CcError;
#[cfg(test)]
use super::types::Engine;
use super::types::{Session, SessionStatus};
use crate::infra::tmux;
use crate::shared::cache;

mod archive;
mod database;

pub(crate) use archive::{ArchiveLogContext, archive_codex_thread_before_delete_with};
pub(crate) use database::{SessionLock, SessionStoreChanges, WakeLock, serialize_session};

/// Threshold in seconds for sort stability.
/// Sessions updated within this window are sorted by created_at instead,
/// preventing rapid reordering during concurrent agent execution.
pub(crate) const SORT_STABILITY_THRESHOLD_SECS: i64 = 30;

/// Retention period for ended sessions before garbage collection.
/// Ended sessions are kept so that `claude -c` resume can restore
/// label and ancestor chain, then cleaned up after this period.
const ENDED_SESSION_RETENTION_DAYS: i64 = 7;

/// Returns the directory for storing Claude Code session data.
/// Path: ~/.cache/armyknife/cc/sessions/
pub fn sessions_dir() -> Result<PathBuf> {
    cache::base_dir()
        .map(|d| d.join("cc").join("sessions"))
        .ok_or_else(|| CcError::CacheDirNotFound.into())
}

/// Returns the directory for crit lifecycle markers.
pub fn crit_dir() -> Result<PathBuf> {
    cache::base_dir()
        .map(|d| d.join("cc").join("crit"))
        .ok_or_else(|| CcError::CacheDirNotFound.into())
}

/// Returns the file path for storing the last selected session ID.
/// Path: ~/.cache/armyknife/cc/last_selected_session
pub fn last_selected_session_file() -> Result<PathBuf> {
    cache::base_dir()
        .map(|d| d.join("cc").join("last_selected_session"))
        .ok_or_else(|| CcError::CacheDirNotFound.into())
}

/// Saves the last selected session ID to disk.
/// Uses atomic write (temp file + rename) to prevent corruption.
pub fn save_last_selected_session(session_id: &str) -> Result<()> {
    save_last_selected_session_to(&last_selected_session_file()?, session_id)
}

/// Saves the last selected session ID to a specific path.
/// Allows testing with temporary directories.
pub(crate) fn save_last_selected_session_to(path: &Path, session_id: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Atomic write via temp file + rename
    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, session_id)?;
    fs::rename(&temp_path, path)?;
    Ok(())
}

/// Loads the last selected session ID from disk.
/// Returns Ok(None) if the file doesn't exist or is empty.
pub fn load_last_selected_session() -> Result<Option<String>> {
    load_last_selected_session_from(&last_selected_session_file()?)
}

/// Loads the last selected session ID from a specific path.
/// Allows testing with temporary directories.
pub(crate) fn load_last_selected_session_from(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)?;
    let session_id = content.trim().to_string();
    if session_id.is_empty() {
        return Ok(None);
    }
    Ok(Some(session_id))
}

/// Loads a session by ID from the session database.
/// Returns Ok(None) if the record doesn't exist.
///
/// If the stored JSON is corrupted, Ok(None) is returned.
pub fn load_session(session_id: &str) -> Result<Option<Session>> {
    let dir = sessions_dir()?;
    load_session_from(&dir, session_id)
}

/// Loads a session from a database in a specific directory.
/// Allows testing with temporary directories.
pub(crate) fn load_session_from(sessions_dir: &Path, session_id: &str) -> Result<Option<Session>> {
    database::load_session(sessions_dir, session_id)
}

/// Saves a session to the database in a specific directory.
/// Allows testing with temporary directories.
pub(crate) fn save_session_to(sessions_dir: &Path, session: &Session) -> Result<()> {
    database::save_session(sessions_dir, session)
}

/// Acquires the exclusive session lock ahead of a load-mutate-save round
/// trip (see `SessionLock`).
pub(crate) fn lock_session_for_update(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<SessionLock> {
    database::lock_session_for_update(sessions_dir, session_id)
}

pub(crate) fn lock_session_for_wake(sessions_dir: &Path, session_id: &str) -> Result<WakeLock> {
    database::lock_session_for_wake(sessions_dir, session_id)
}

/// Loads a session under the write transaction, lets `mutate` decide whether
/// and how to change it, and writes the result back iff `mutate` returns
/// `true`. A no-op when the record is missing or corrupted.
///
/// Holds the exclusive lock across the whole load-modify-save round-trip so
/// a concurrent hook write of the full session (e.g. a new `last_message`)
/// cannot interleave with this one and tear the result. Shared by session
/// updates so the atomic-write plumbing exists once.
pub(crate) fn update_session_in(
    sessions_dir: &Path,
    session_id: &str,
    mutate: impl FnOnce(&mut Session) -> bool,
) -> Result<()> {
    database::update_session(sessions_dir, session_id, mutate)
}

/// Rewrites an existing session so file watchers observe derived status changes.
pub(crate) fn touch_session(session_id: &str) -> Result<()> {
    database::touch_session(&sessions_dir()?, session_id)
}

/// Atomically marks a `Stopped` session as read by setting `read_at = Some(now)`.
/// A no-op when the session is missing, corrupted, no longer `Stopped`, or
/// already marked read — `mark-read` only flips unread→read, never
/// overwrites a fresh Stop.
pub(crate) fn mark_session_read_in(
    sessions_dir: &Path,
    session_id: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    update_session_in(sessions_dir, session_id, |session| {
        if session.status != SessionStatus::Stopped || session.read_at.is_some() {
            return false;
        }
        session.read_at = Some(now);
        true
    })
}

/// Overwrites a session's `last_message` field under a freshly acquired
/// transaction, but only if its current stored value still equals
/// `expected_current` (the value read before `SessionLock` was released for
/// the hook handler's possibly slow transcript read -- see that struct's
/// doc comment for why the read cannot happen while the lock is held).
/// Without this compare-and-swap, a slow transcript read from one hook event
/// could overwrite a fresher `last_message` already saved by a later event
/// on the same session.
pub(crate) fn update_session_last_message_if_unchanged_in(
    sessions_dir: &Path,
    session_id: &str,
    expected_current: Option<&str>,
    last_message: Option<String>,
) -> Result<()> {
    update_session_in(sessions_dir, session_id, |session| {
        if session.last_message.as_deref() != expected_current {
            return false;
        }
        session.last_message = last_message;
        true
    })
}

/// Sets `label` on a session, resolving the default sessions directory.
/// Thin wrapper so callers (e.g. the TUI's rename key) do not need to know
/// about `sessions_dir()`.
pub fn update_session_label(session_id: &str, label: Option<String>) -> Result<()> {
    update_session_label_in(&sessions_dir()?, session_id, label)
}

/// Atomically overwrites a session's `label` field. Unlike
/// `mark_session_read_in` there is no precondition on the current value --
/// `label` is always overwritten with the given value.
pub(crate) fn update_session_label_in(
    sessions_dir: &Path,
    session_id: &str,
    label: Option<String>,
) -> Result<()> {
    update_session_in(sessions_dir, session_id, |session| {
        session.label = label;
        true
    })
}

/// Sets `label` on a session, but only if its current on-disk value still
/// equals `expected_current`. Resolves the default sessions directory --
/// see `update_session_label_if_unchanged_in` for the full contract.
pub fn update_session_label_if_unchanged(
    session_id: &str,
    expected_current: Option<&str>,
    new_label: Option<String>,
) -> Result<bool> {
    update_session_label_if_unchanged_in(&sessions_dir()?, session_id, expected_current, new_label)
}

/// Compare-and-swap variant of `update_session_label_in`: applies
/// `new_label` only when the session's current `label` still equals
/// `expected_current`, so a manual rename made after `expected_current` was
/// snapshotted is never clobbered by a stale write (e.g. a detached title
/// generation that finishes after the user renamed the session by hand).
/// Returns whether the write happened -- `false` covers both a mismatched
/// label and a missing session record.
pub(crate) fn update_session_label_if_unchanged_in(
    sessions_dir: &Path,
    session_id: &str,
    expected_current: Option<&str>,
    new_label: Option<String>,
) -> Result<bool> {
    let mut applied = false;
    update_session_in(sessions_dir, session_id, |session| {
        if session.label.as_deref() != expected_current {
            return false;
        }
        session.label = new_label;
        applied = true;
        true
    })?;
    Ok(applied)
}

/// Rebinds a session to a restored tmux pane and clears its archive marker.
/// The archive marker is cleared even when `tmux_info` is unset.
///
/// tmux-resurrect restarts the tmux server, renumbering every pane_id, so a
/// stale on-disk pane_id can coincidentally match an unrelated session's new
/// pane. `resurrect::run_restore` calls this to keep
/// `evict_paused_sessions_on_pane_takeover` (hook.rs) from mismatching on it.
pub(crate) fn update_session_tmux_pane_id_in(
    sessions_dir: &Path,
    session_id: &str,
    pane_id: &str,
) -> Result<()> {
    update_session_in(sessions_dir, session_id, |session| {
        let mut changed = session.restore_from_archive();
        let Some(tmux_info) = session.tmux_info.as_mut() else {
            return changed;
        };
        if tmux_info.pane_id != pane_id {
            tmux_info.pane_id = pane_id.to_string();
            changed = true;
        }
        changed
    })
}

/// Archives a Codex thread before deleting its session record.
/// Returns Ok(()) even if the record doesn't exist or archiving fails.
pub fn delete_session(session_id: &str) -> Result<()> {
    archive::delete_session_with_archive(session_id)
}

/// Deletes a session without archiving its Codex thread.
/// Used when cleanup handles archive ordering separately.
pub(crate) fn delete_session_without_archive(session_id: &str) -> Result<()> {
    delete_session_from(&sessions_dir()?, session_id)
}

/// Deletes a session from a specific directory.
/// Allows testing with temporary directories.
pub(crate) fn delete_session_from(sessions_dir: &Path, session_id: &str) -> Result<()> {
    database::delete_session(sessions_dir, session_id)?;
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

/// Lists all active sessions from disk, excluding ended sessions.
pub fn list_sessions() -> Result<Vec<Session>> {
    let mut sessions = database::all_sessions(&sessions_dir()?)?;
    sessions.retain(|session| session.archived_at.is_none());
    sessions.retain(|s| s.status != SessionStatus::Ended);
    sort_sessions(&mut sessions);
    Ok(sessions)
}

/// Lists every non-archived session from disk, including ended ones. Unlike
/// `list_sessions`, callers use this when a session must be resolvable
/// after it ended -- e.g. reading `ancestor_session_ids` off a delegated
/// session that already exited by the time its PR merged.
pub fn list_all_sessions() -> Result<Vec<Session>> {
    list_all_sessions_from(&sessions_dir()?)
}

/// Lists every non-archived session from a specific directory, including ended ones.
pub(crate) fn list_all_sessions_from(sessions_dir: &Path) -> Result<Vec<Session>> {
    let mut sessions = database::all_sessions(sessions_dir)?;
    sessions.retain(|session| session.archived_at.is_none());
    sort_sessions(&mut sessions);
    Ok(sessions)
}

pub(crate) fn session_store_revision() -> Result<i64> {
    session_store_revision_from(&sessions_dir()?)
}

pub(crate) fn session_store_revision_from(sessions_dir: &Path) -> Result<i64> {
    database::session_store_revision(sessions_dir)
}

pub(crate) fn session_changes_since(
    sessions_dir: &Path,
    revision: i64,
) -> Result<SessionStoreChanges> {
    database::session_changes_since(sessions_dir, revision)
}

/// Archives stale sessions and removes records whose retention period expired.
///
/// A session is considered stale if its tmux pane no longer exists.
/// Sessions without tmux_info are kept (they may be running outside tmux).
/// Sessions are checked against their recorded server; failed server queries
/// leave those sessions untouched.
pub fn cleanup_stale_sessions() -> Result<bool> {
    // `Path::exists()` also returns false on ambiguous errors (e.g. permission
    // denied, a transient mount issue), not just on confirmed deletion. Treat
    // only a confirmed NotFound as "gone" so a live session's file is never
    // deleted because of a stat() error that isn't really about the cwd
    // being removed.
    let cwd_exists =
        |cwd: &Path| !matches!(fs::metadata(cwd), Err(e) if e.kind() == io::ErrorKind::NotFound);
    cleanup_stale_sessions_by_socket(
        &sessions_dir()?,
        tmux::list_all_pane_ids_for_socket,
        cwd_exists,
    )
}

fn cleanup_stale_sessions_by_socket<F, G>(dir: &Path, list_panes: F, cwd_exists: G) -> Result<bool>
where
    F: FnMut(Option<&str>) -> tmux::Result<HashSet<String>>,
    G: Fn(&Path) -> bool,
{
    let cache = RefCell::new(tmux::PaneLiveness::default());
    let list_panes = RefCell::new(list_panes);
    let is_pane_alive = |info: &super::types::TmuxInfo| {
        cache
            .borrow_mut()
            .cache_socket(info.socket_path.as_deref(), |socket_path| {
                (list_panes.borrow_mut())(socket_path)
            });
        cache
            .borrow()
            .is_alive(info.socket_path.as_deref(), &info.pane_id)
    };
    cleanup_stale_sessions_in(dir, is_pane_alive, cwd_exists)
}

/// Returns whether cleanup archived or removed at least one session record.
fn cleanup_stale_sessions_in<F, G>(dir: &Path, is_pane_alive: F, cwd_exists: G) -> Result<bool>
where
    F: Fn(&super::types::TmuxInfo) -> Option<bool>,
    G: Fn(&Path) -> bool,
{
    cleanup_stale_sessions_in_with_archive(
        dir,
        is_pane_alive,
        cwd_exists,
        crate::commands::agent::codex_steer::archive_thread,
    )
}

fn cleanup_stale_sessions_in_with_archive<F, G, A>(
    dir: &Path,
    is_pane_alive: F,
    cwd_exists: G,
    archive_codex_thread: A,
) -> Result<bool>
where
    F: Fn(&super::types::TmuxInfo) -> Option<bool>,
    G: Fn(&Path) -> bool,
    A: FnMut(&str) -> anyhow::Result<()>,
{
    archive::cleanup_stale_sessions_in_with_archive_at(
        dir,
        is_pane_alive,
        cwd_exists,
        Utc::now(),
        archive_codex_thread,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::SessionStatus;
    use chrono::{TimeDelta, TimeZone, Utc};
    use rstest::{fixture, rstest};
    use rusqlite::OptionalExtension;
    use std::fs::OpenOptions;
    use tempfile::TempDir;

    struct TempSessionDir {
        #[expect(dead_code, reason = "kept alive to prevent cleanup until dropped")]
        temp_dir: TempDir,
        sessions_path: PathBuf,
    }

    struct StoredSessionProbe {
        sessions_dir: PathBuf,
        session_id: String,
        legacy_json_path: PathBuf,
    }

    impl StoredSessionProbe {
        fn is_stored(&self) -> bool {
            database::load_session(&self.sessions_dir, &self.session_id)
                .ok()
                .flatten()
                .is_some()
        }

        fn legacy_json_path(&self) -> &Path {
            &self.legacy_json_path
        }

        fn legacy_json_lock_path(&self) -> PathBuf {
            self.legacy_json_path.with_extension("json.lock")
        }
    }

    fn session_record_in(sessions_dir: &Path, session_id: &str) -> Result<StoredSessionProbe> {
        database::validate_session_id(session_id)?;
        Ok(StoredSessionProbe {
            sessions_dir: sessions_dir.to_path_buf(),
            session_id: session_id.to_string(),
            legacy_json_path: sessions_dir.join(format!("{session_id}.json")),
        })
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
            archived_at: None,
            pr_context: None,
            session_id: id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            is_delegated: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            pending_agent_task_ids: std::collections::BTreeSet::new(),
            pending_permission_agent_ids: std::collections::BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Engine::Claude,
        }
    }

    #[test]
    fn list_sessions_excludes_ended_and_both_lists_exclude_archived_sessions() {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let cache_home = temp_dir.path().to_str().expect("utf8 path").to_string();
        let sessions_path = temp_dir
            .path()
            .join("armyknife")
            .join("cc")
            .join("sessions");

        let mut ended = create_test_session("ended-1");
        ended.status = SessionStatus::Ended;
        let running = create_test_session("running-1");
        let archived_at = Utc
            .with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
            .single()
            .expect("timestamp should be valid");
        let mut archived = create_test_session("archived-1");
        archived.archived_at = Some(archived_at);
        save_session_to(&sessions_path, &ended).expect("save should succeed");
        save_session_to(&sessions_path, &running).expect("save should succeed");
        save_session_to(&sessions_path, &archived).expect("save should succeed");

        let (mut active_ids, mut all_ids, archived_record) =
            temp_env::with_vars([("XDG_CACHE_HOME", Some(cache_home.as_str()))], || {
                let active: Vec<String> = list_sessions()
                    .expect("list_sessions should succeed")
                    .into_iter()
                    .map(|s| s.session_id)
                    .collect();
                let all: Vec<String> = list_all_sessions()
                    .expect("list_all_sessions should succeed")
                    .into_iter()
                    .map(|s| s.session_id)
                    .collect();
                let archived_record =
                    load_session_from(&sessions_path, "archived-1").expect("load should succeed");
                (
                    active,
                    all,
                    archived_record.map(|session| session.archived_at),
                )
            });
        active_ids.sort();
        all_ids.sort();

        assert_eq!(
            (active_ids, all_ids, archived_record),
            (
                vec!["running-1".to_string()],
                vec!["ended-1".to_string(), "running-1".to_string()],
                Some(Some(archived_at)),
            )
        );
    }

    #[rstest::rstest]
    fn list_all_sessions_from_skips_archived_and_malformed_sessions(
        temp_session_dir: TempSessionDir,
    ) {
        let running = create_test_session("running-session");
        let mut ended = create_test_session("ended-session");
        ended.status = SessionStatus::Ended;
        let mut archived = create_test_session("archived-session");
        archived.archived_at = Some(Utc::now());
        save_session_to(&temp_session_dir.sessions_path, &running).expect("save running session");
        save_session_to(&temp_session_dir.sessions_path, &ended).expect("save ended session");
        save_session_to(&temp_session_dir.sessions_path, &archived).expect("save archived session");
        fs::write(
            temp_session_dir
                .sessions_path
                .join("malformed-session.json"),
            "not json",
        )
        .expect("write malformed session");

        let mut actual = list_all_sessions_from(&temp_session_dir.sessions_path)
            .expect("list should succeed")
            .into_iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>();
        actual.sort();

        assert_eq!(
            actual,
            vec!["ended-session".to_string(), "running-session".to_string()]
        );
    }

    #[rstest::rstest]
    fn list_all_sessions_from_adds_context_when_database_directory_cannot_be_created(
        temp_session_dir: TempSessionDir,
    ) {
        let file_path = temp_session_dir.sessions_path.join("not-a-directory");
        fs::write(&file_path, "").expect("write file");

        let error = list_all_sessions_from(&file_path).expect_err("file is not a directory");

        assert_eq!(
            error.to_string(),
            format!("creating sessions dir {}", file_path.display())
        );
    }

    #[test]
    fn test_session_serialization() {
        let session = create_test_session("test-123");
        let json = serde_json::to_string(&session).expect("serialization should succeed");
        let parsed: Session = serde_json::from_str(&json).expect("deserialization should succeed");

        assert_eq!(parsed.session_id, "test-123");
        assert_eq!(parsed.status, SessionStatus::Running);
    }

    #[rstest::rstest]
    fn imports_legacy_json_and_removes_its_lock_file(temp_session_dir: TempSessionDir) {
        let session = create_test_session("import-target");
        let json_path = temp_session_dir.sessions_path.join("import-target.json");
        let lock_path = temp_session_dir
            .sessions_path
            .join("import-target.json.lock");
        let backup_path = temp_session_dir
            .sessions_path
            .join("legacy-json")
            .join("import-target.json");
        fs::write(
            &json_path,
            serde_json::to_vec_pretty(&session).expect("session should serialize"),
        )
        .expect("legacy session should write");
        fs::write(&lock_path, "").expect("legacy lock file should write");

        let imported = load_session_from(&temp_session_dir.sessions_path, "import-target")
            .expect("legacy import should succeed")
            .expect("imported session should load");
        let actual = (
            serde_json::to_value(imported).expect("imported session should serialize"),
            json_path.exists(),
            backup_path.exists(),
            lock_path.exists(),
        );
        let expected = (
            serde_json::to_value(session).expect("expected session should serialize"),
            false,
            true,
            false,
        );

        assert_eq!(actual, expected);
    }

    #[rstest::rstest]
    fn legacy_import_prefers_newer_updated_at_values(temp_session_dir: TempSessionDir) {
        let newer_at = Utc
            .with_ymd_and_hms(2026, 4, 2, 12, 0, 0)
            .single()
            .expect("timestamp should be valid");
        let older_at = newer_at - TimeDelta::hours(1);
        let mut stored = create_test_session("updated-target");
        stored.updated_at = newer_at;
        stored.label = Some("stored value".to_string());
        save_session_to(&temp_session_dir.sessions_path, &stored).expect("stored row should save");

        let mut legacy = stored.clone();
        legacy.updated_at = older_at;
        legacy.label = Some("older JSON value".to_string());
        let json_path = temp_session_dir.sessions_path.join("updated-target.json");
        fs::write(
            &json_path,
            serde_json::to_vec(&legacy).expect("legacy session should serialize"),
        )
        .expect("legacy session should write");
        let after_older_import =
            load_session_from(&temp_session_dir.sessions_path, "updated-target")
                .expect("older legacy import should succeed")
                .expect("stored row should remain");

        legacy.updated_at = newer_at + TimeDelta::hours(1);
        legacy.label = Some("newer JSON value".to_string());
        fs::write(
            &json_path,
            serde_json::to_vec(&legacy).expect("newer legacy session should serialize"),
        )
        .expect("newer legacy session should write");
        let after_newer_import =
            load_session_from(&temp_session_dir.sessions_path, "updated-target")
                .expect("newer legacy import should succeed")
                .expect("newer row should load");

        assert_eq!(
            (
                after_older_import.updated_at,
                after_older_import.label,
                after_newer_import.updated_at,
                after_newer_import.label,
            ),
            (
                newer_at,
                Some("stored value".to_string()),
                newer_at + TimeDelta::hours(1),
                Some("newer JSON value".to_string()),
            ),
        );
    }

    #[rstest::rstest]
    fn removes_legacy_backup_after_thirty_days(temp_session_dir: TempSessionDir) {
        let session = create_test_session("retained-legacy");
        let json_path = temp_session_dir.sessions_path.join("retained-legacy.json");
        fs::write(
            &json_path,
            serde_json::to_vec(&session).expect("legacy session should serialize"),
        )
        .expect("legacy session should write");
        load_session_from(&temp_session_dir.sessions_path, "retained-legacy")
            .expect("legacy import should succeed");
        let backup_dir = temp_session_dir.sessions_path.join("legacy-json");
        let connection =
            rusqlite::Connection::open(temp_session_dir.sessions_path.join("sessions.db"))
                .expect("database should open");
        connection
            .execute(
                "UPDATE session_store_meta SET value = ?1 WHERE key = 'legacy-imported-at'",
                [(Utc::now() - TimeDelta::days(31)).to_rfc3339()],
            )
            .expect("legacy import time should update");
        drop(connection);

        list_all_sessions_from(&temp_session_dir.sessions_path)
            .expect("opening the database should clean expired backups");

        assert!(!backup_dir.exists());
    }

    #[rstest::rstest]
    fn late_legacy_import_refreshes_backup_retention(temp_session_dir: TempSessionDir) {
        let initial = create_test_session("existing-row");
        save_session_to(&temp_session_dir.sessions_path, &initial).expect("row should save");
        let backup_dir = temp_session_dir.sessions_path.join("legacy-json");
        fs::create_dir_all(&backup_dir).expect("legacy backup directory should exist");
        let earlier_backup = backup_dir.join("earlier-import.json");
        fs::write(&earlier_backup, "retained backup").expect("earlier backup should write");
        let connection =
            rusqlite::Connection::open(temp_session_dir.sessions_path.join("sessions.db"))
                .expect("database should open");
        connection
            .execute(
                "INSERT INTO session_store_meta (key, value) VALUES ('legacy-imported-at', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [(Utc::now() - TimeDelta::days(31)).to_rfc3339()],
            )
            .expect("legacy import time should update");
        drop(connection);

        let late_session = create_test_session("late-import");
        let late_json = temp_session_dir.sessions_path.join("late-import.json");
        fs::write(
            &late_json,
            serde_json::to_vec(&late_session).expect("legacy session should serialize"),
        )
        .expect("late legacy session should write");
        let loaded = load_session_from(&temp_session_dir.sessions_path, "late-import")
            .expect("late legacy import should succeed")
            .expect("late session should load");

        assert_eq!(
            (
                loaded.session_id,
                earlier_backup.exists(),
                backup_dir.join("late-import.json").exists(),
            ),
            ("late-import".to_string(), true, true),
        );
    }

    #[rstest::rstest]
    fn preserves_active_orphaned_legacy_lock_files(temp_session_dir: TempSessionDir) {
        let lock_path = temp_session_dir
            .sessions_path
            .join("creating-session.json.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .expect("legacy lock should open");
        lock.try_lock().expect("legacy lock should be acquired");

        database::open_connection(&temp_session_dir.sessions_path)
            .expect("database should open while the legacy writer holds its lock");
        let lock_exists_while_owned = lock_path.exists();
        drop(lock);

        database::open_connection(&temp_session_dir.sessions_path)
            .expect("database should open after the legacy writer releases its lock");

        assert_eq!((lock_exists_while_owned, lock_path.exists()), (true, false));
    }

    const UNDECODABLE_SESSION_ID: &str = "undecodable-session";

    struct UndecodableSession {
        temp_session_dir: TempSessionDir,
        data: String,
    }

    #[fixture]
    fn undecodable_session(temp_session_dir: TempSessionDir) -> UndecodableSession {
        let data = "{not-json".to_string();
        let connection = database::open_connection(&temp_session_dir.sessions_path)
            .expect("session database should open");
        connection
            .execute(
                "INSERT INTO sessions (session_id, data, rev) VALUES (?1, ?2, 7)",
                rusqlite::params![UNDECODABLE_SESSION_ID, data],
            )
            .expect("undecodable session row should insert");
        connection
            .execute(
                "UPDATE session_store_meta SET value = '7' WHERE key = 'revision'",
                [],
            )
            .expect("revision should match inserted row");
        UndecodableSession {
            temp_session_dir,
            data,
        }
    }

    #[rstest::rstest]
    fn session_lock_load_reports_undecodable_records(undecodable_session: UndecodableSession) {
        let lock = lock_session_for_update(
            &undecodable_session.temp_session_dir.sessions_path,
            UNDECODABLE_SESSION_ID,
        )
        .expect("session lock should open");
        let actual = lock.load().err().map(|error| error.to_string());

        assert_eq!(
            actual,
            Some(format!(
                "session {UNDECODABLE_SESSION_ID} is stored but its data cannot be decoded"
            )),
        );
    }

    #[rstest::rstest]
    fn load_skips_undecodable_records(undecodable_session: UndecodableSession) {
        let actual = load_session_from(
            &undecodable_session.temp_session_dir.sessions_path,
            UNDECODABLE_SESSION_ID,
        )
        .expect("reading an undecodable session should succeed")
        .map(|session| session.session_id);

        assert_eq!(actual, None);
    }

    #[rstest::rstest]
    fn save_does_not_overwrite_undecodable_records(undecodable_session: UndecodableSession) {
        let sessions_dir = &undecodable_session.temp_session_dir.sessions_path;
        let save_result =
            save_session_to(sessions_dir, &create_test_session(UNDECODABLE_SESSION_ID));
        let connection = rusqlite::Connection::open(sessions_dir.join("sessions.db"))
            .expect("session database should open");
        let stored_row = connection
            .query_row(
                "SELECT data, rev FROM sessions WHERE session_id = ?1",
                [UNDECODABLE_SESSION_ID],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .expect("undecodable row should remain stored");
        let revision = connection
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM session_store_meta WHERE key = 'revision'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("store revision should remain readable");

        assert_eq!(
            (save_result.is_ok(), stored_row, revision),
            (false, (undecodable_session.data, 7), 7),
        );
    }

    #[rstest::rstest]
    fn update_skips_undecodable_records(undecodable_session: UndecodableSession) {
        let sessions_dir = &undecodable_session.temp_session_dir.sessions_path;
        let update_result = update_session_in(sessions_dir, UNDECODABLE_SESSION_ID, |session| {
            session.label = Some("replacement".to_string());
            true
        });
        let connection = rusqlite::Connection::open(sessions_dir.join("sessions.db"))
            .expect("session database should open");
        let stored_row = connection
            .query_row(
                "SELECT data, rev FROM sessions WHERE session_id = ?1",
                [UNDECODABLE_SESSION_ID],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .expect("undecodable row should remain stored");
        let revision = connection
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM session_store_meta WHERE key = 'revision'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("store revision should remain readable");

        assert_eq!(
            (update_result.is_ok(), stored_row, revision),
            (true, (undecodable_session.data, 7), 7),
        );
    }

    #[rstest::rstest]
    fn legacy_import_skips_undecodable_database_rows(undecodable_session: UndecodableSession) {
        let sessions_dir = &undecodable_session.temp_session_dir.sessions_path;
        let legacy_json = sessions_dir.join(format!("{UNDECODABLE_SESSION_ID}.json"));
        fs::write(
            &legacy_json,
            serde_json::to_vec(&create_test_session(UNDECODABLE_SESSION_ID))
                .expect("legacy session should serialize"),
        )
        .expect("legacy session should write");

        let importable_session = create_test_session("valid-legacy-session");
        let importable_legacy_json = sessions_dir.join("valid-legacy-session.json");
        fs::write(
            &importable_legacy_json,
            serde_json::to_vec(&importable_session).expect("legacy session should serialize"),
        )
        .expect("legacy session should write");

        let connection = database::open_connection(sessions_dir)
            .expect("database should open when legacy data conflicts with an undecodable row");
        let stored_data = connection
            .query_row(
                "SELECT data FROM sessions WHERE session_id = ?1",
                [UNDECODABLE_SESSION_ID],
                |row| row.get::<_, String>(0),
            )
            .expect("undecodable row should remain stored");
        let backup_path = sessions_dir
            .join("legacy-json")
            .join(format!("{UNDECODABLE_SESSION_ID}.json"));
        let imported_data: Option<String> = connection
            .query_row(
                "SELECT data FROM sessions WHERE session_id = ?1",
                [importable_session.session_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .expect("valid legacy session should be queryable");
        let importable_backup_path = sessions_dir
            .join("legacy-json")
            .join("valid-legacy-session.json");

        assert_eq!(
            (
                stored_data,
                legacy_json.exists(),
                backup_path.exists(),
                imported_data,
                importable_legacy_json.exists(),
                importable_backup_path.exists(),
            ),
            (
                undecodable_session.data,
                true,
                false,
                Some(
                    database::serialize_session(&importable_session)
                        .expect("legacy session should serialize"),
                ),
                false,
                true,
            ),
        );
    }

    #[rstest::rstest]
    fn database_open_succeeds_when_legacy_import_fails(temp_session_dir: TempSessionDir) {
        let legacy_json = temp_session_dir.sessions_path.join("blocked-import.json");
        fs::write(&legacy_json, "invalid legacy record").expect("legacy JSON should write");
        fs::create_dir(legacy_json.with_extension("json.lock"))
            .expect("legacy lock path should become unreadable");

        let connection = database::open_connection(&temp_session_dir.sessions_path)
            .expect("legacy import failure should not prevent opening the database");
        let row_count = connection
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("database should remain usable");

        assert_eq!(row_count, 0);
    }

    #[test]
    fn test_save_and_load_session() {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let sessions_path = temp_dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_path).expect("dir creation should succeed");

        let session = create_test_session("save-load-test");
        save_session_to(&sessions_path, &session).expect("save should succeed");
        let loaded = load_session_from(&sessions_path, "save-load-test")
            .expect("load should succeed")
            .expect("session should exist");

        assert_eq!(
            (loaded.session_id, loaded.status),
            ("save-load-test".to_string(), SessionStatus::Running),
        );
    }

    #[rstest]
    #[case::empty_id("", false)]
    #[case::parent_traversal("../etc/passwd", false)]
    #[case::nested_path("foo/bar", false)]
    #[case::windows_nested_path("foo\\bar", false)]
    #[case::dot_dot("..", false)]
    #[case::valid_hyphenated_id("valid-session-id", true)]
    #[case::valid_underscored_id("session_123", true)]
    fn session_store_rejects_invalid_ids_at_its_api(
        temp_session_dir: TempSessionDir,
        #[case] session_id: &str,
        #[case] expected_to_load: bool,
    ) {
        let actual_to_load = load_session_from(&temp_session_dir.sessions_path, session_id).is_ok();

        assert_eq!(actual_to_load, expected_to_load);
    }

    mod session_lock_tests {
        use super::*;
        use rstest::rstest;
        use std::sync::{Arc, Barrier};

        #[rstest]
        fn load_then_save_round_trips_a_mutation(temp_session_dir: TempSessionDir) {
            let session = create_test_session("lock-test");
            save_session_to(&temp_session_dir.sessions_path, &session).expect("save");

            let lock = lock_session_for_update(&temp_session_dir.sessions_path, "lock-test")
                .expect("lock");
            let mut loaded = lock.load().expect("load").expect("session exists");
            loaded
                .pending_permission_agent_ids
                .insert("agent-a".to_string());
            lock.save(&loaded).expect("save");
            drop(lock);

            let reloaded = load_session_from(&temp_session_dir.sessions_path, "lock-test")
                .expect("load")
                .expect("session exists");
            assert_eq!(
                reloaded.pending_permission_agent_ids,
                std::collections::BTreeSet::from(["agent-a".to_string()])
            );
        }

        #[rstest]
        fn concurrent_updates_do_not_overwrite_same_or_different_sessions(
            temp_session_dir: TempSessionDir,
        ) {
            let sessions_dir = temp_session_dir.sessions_path.clone();
            let worker_count = 8;
            save_session_to(&sessions_dir, &create_test_session("shared-row"))
                .expect("shared row should save");
            let worker_ids = (0..worker_count)
                .map(|index| format!("worker-{index}"))
                .collect::<Vec<_>>();
            for worker_id in &worker_ids {
                save_session_to(&sessions_dir, &create_test_session(worker_id))
                    .expect("worker row should save");
            }

            let barrier = Arc::new(Barrier::new(worker_count + 1));
            let workers = worker_ids
                .iter()
                .map(|worker_id| {
                    let sessions_dir = sessions_dir.clone();
                    let worker_id = worker_id.clone();
                    let barrier = Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        barrier.wait();
                        update_session_in(&sessions_dir, "shared-row", |session| {
                            session
                                .pending_permission_agent_ids
                                .insert(worker_id.clone());
                            true
                        })
                        .expect("shared row update should succeed");
                        update_session_in(&sessions_dir, &worker_id, |session| {
                            session
                                .pending_permission_agent_ids
                                .insert(worker_id.clone());
                            true
                        })
                        .expect("independent row update should succeed");
                    })
                })
                .collect::<Vec<_>>();
            barrier.wait();
            for worker in workers {
                worker.join().expect("writer thread should complete");
            }

            let shared = load_session_from(&sessions_dir, "shared-row")
                .expect("shared row should load")
                .expect("shared row should exist")
                .pending_permission_agent_ids;
            let independent = worker_ids
                .iter()
                .map(|worker_id| {
                    load_session_from(&sessions_dir, worker_id)
                        .expect("worker row should load")
                        .expect("worker row should exist")
                        .pending_permission_agent_ids
                })
                .collect::<Vec<_>>();
            let expected_shared = worker_ids
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            let expected_independent = worker_ids
                .iter()
                .map(|worker_id| std::collections::BTreeSet::from([worker_id.clone()]))
                .collect::<Vec<_>>();

            assert_eq!(
                (shared, independent),
                (expected_shared, expected_independent)
            );
        }

        #[rstest]
        fn touch_advances_revision_without_changing_session_data(temp_session_dir: TempSessionDir) {
            let session = create_test_session("touch-row");
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("session should save");
            let before_revision = database::session_store_revision(&temp_session_dir.sessions_path)
                .expect("revision should load");
            let before = serde_json::to_value(
                load_session_from(&temp_session_dir.sessions_path, "touch-row")
                    .expect("session should load")
                    .expect("session should exist"),
            )
            .expect("session should serialize");

            database::touch_session(&temp_session_dir.sessions_path, "touch-row")
                .expect("touch should succeed");

            let after = serde_json::to_value(
                load_session_from(&temp_session_dir.sessions_path, "touch-row")
                    .expect("session should load")
                    .expect("session should exist"),
            )
            .expect("session should serialize");
            let after_revision = database::session_store_revision(&temp_session_dir.sessions_path)
                .expect("revision should load");
            assert_eq!(
                (before == after, after_revision),
                (true, before_revision + 1),
            );
        }
    }

    mod cleanup_stale_sessions_tests {
        use super::*;
        use crate::commands::agent::types::TmuxInfo;
        use rstest::rstest;

        /// Mock that treats all panes as dead
        fn mock_pane_always_dead(_info: &TmuxInfo) -> Option<bool> {
            Some(false)
        }

        /// Mock that treats all panes as alive
        fn mock_pane_always_alive(_info: &TmuxInfo) -> Option<bool> {
            Some(true)
        }

        /// Mock that treats every cwd as existing on disk
        fn mock_cwd_always_exists(_cwd: &Path) -> bool {
            true
        }

        /// Mock that treats every cwd as missing from disk
        fn mock_cwd_always_missing(_cwd: &Path) -> bool {
            false
        }

        #[rstest]
        #[case::running(SessionStatus::Running)]
        #[case::paused(SessionStatus::Paused)]
        fn archives_session_with_dead_pane(
            temp_session_dir: TempSessionDir,
            #[case] status: SessionStatus,
        ) {
            let session_id = "dead-pane-test";
            let path = session_record_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_record_in should succeed");

            let mut session = create_test_session(session_id);
            session.status = status;
            session.tmux_info = Some(TmuxInfo {
                session_name: "test".to_string(),
                window_name: "test".to_string(),
                window_index: 0,
                pane_id: "%99999".to_string(),
                socket_path: None,
            });
            session.label = Some("retained label".to_string());
            session.crit_urls = vec!["https://example.invalid/review".to_string()];
            session.ancestor_session_ids = vec!["parent-session".to_string()];
            session
                .pending_bg_task_ids
                .insert("background-task".to_string());
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");
            assert!(
                path.is_stored(),
                "session record should exist before cleanup"
            );

            let archived_at = Utc
                .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("timestamp should be valid");
            let changed = archive::cleanup_stale_sessions_in_with_archive_at(
                &temp_session_dir.sessions_path,
                mock_pane_always_dead,
                mock_cwd_always_exists,
                archived_at,
                |_| Ok(()),
            )
            .expect("cleanup should succeed");

            let restored = load_session_from(&temp_session_dir.sessions_path, session_id)
                .expect("load should succeed")
                .map(|session| {
                    (
                        session.archived_at,
                        session.label,
                        session.crit_urls,
                        session.ancestor_session_ids,
                        session.pending_bg_task_ids,
                    )
                });
            assert_eq!(
                (changed, path.is_stored(), restored),
                (
                    true,
                    true,
                    Some((
                        Some(archived_at),
                        Some("retained label".to_string()),
                        vec!["https://example.invalid/review".to_string()],
                        vec!["parent-session".to_string()],
                        std::collections::BTreeSet::from(["background-task".to_string()]),
                    )),
                ),
            );
        }

        #[rstest]
        fn archives_codex_thread_only_when_archived_session_is_collected(
            temp_session_dir: TempSessionDir,
        ) {
            let session_id = "stale-codex-session";
            let path = session_record_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_record_in should succeed");
            let mut session = create_test_session(session_id);
            session.engine = Engine::Codex;
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");
            let archive_calls = std::cell::RefCell::new(Vec::new());
            let archived_at = Utc
                .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("timestamp should be valid");

            let first_changed = archive::cleanup_stale_sessions_in_with_archive_at(
                &temp_session_dir.sessions_path,
                mock_pane_always_alive,
                mock_cwd_always_missing,
                archived_at,
                |_| Ok(()),
            )
            .expect("cleanup should succeed");
            let archived_record = load_session_from(&temp_session_dir.sessions_path, session_id)
                .expect("load should succeed")
                .map(|session| session.archived_at);
            let calls_before_collection = archive_calls.borrow().clone();

            let removed_after_retention = archive::cleanup_stale_sessions_in_with_archive_at(
                &temp_session_dir.sessions_path,
                mock_pane_always_alive,
                mock_cwd_always_exists,
                archived_at + TimeDelta::days(8),
                |thread_id| {
                    let lock_available =
                        lock_session_for_update(&temp_session_dir.sessions_path, thread_id).is_ok();
                    archive_calls.borrow_mut().push((
                        thread_id.to_string(),
                        path.is_stored(),
                        lock_available,
                    ));
                    Err(anyhow::anyhow!("fixture archive failure"))
                },
            )
            .expect("cleanup should continue after archive failure");

            assert_eq!(
                (
                    first_changed,
                    archived_record,
                    calls_before_collection,
                    removed_after_retention,
                    archive_calls.into_inner(),
                    path.is_stored(),
                ),
                (
                    true,
                    Some(Some(archived_at)),
                    Vec::new(),
                    true,
                    vec![("stale-codex-session".to_string(), true, true)],
                    false,
                ),
            );
        }

        #[rstest]
        fn keeps_session_restored_during_codex_archiving(temp_session_dir: TempSessionDir) {
            let session_id = "restored-during-gc";
            let path = session_record_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_record_in should succeed");
            let now = Utc
                .with_ymd_and_hms(2026, 1, 10, 0, 0, 0)
                .single()
                .expect("timestamp should be valid");
            let mut session = create_test_session(session_id);
            session.engine = Engine::Codex;
            session.archived_at = Some(now - TimeDelta::days(8));
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");
            let archive_calls = std::cell::RefCell::new(Vec::new());

            let changed = archive::cleanup_stale_sessions_in_with_archive_at(
                &temp_session_dir.sessions_path,
                mock_pane_always_alive,
                mock_cwd_always_exists,
                now,
                |thread_id| {
                    archive_calls.borrow_mut().push(thread_id.to_string());
                    update_session_in(&temp_session_dir.sessions_path, thread_id, |session| {
                        session.archived_at = None;
                        session.status = SessionStatus::Running;
                        session.updated_at = now;
                        true
                    })?;
                    Ok(())
                },
            )
            .expect("cleanup should preserve a session restored during archiving");

            let restored = load_session_from(&temp_session_dir.sessions_path, session_id)
                .expect("load should succeed")
                .map(|session| (session.archived_at, session.status));
            assert_eq!(
                (
                    changed,
                    archive_calls.into_inner(),
                    path.is_stored(),
                    restored
                ),
                (
                    false,
                    vec!["restored-during-gc".to_string()],
                    true,
                    Some((None, SessionStatus::Running)),
                ),
            );
        }

        #[rstest]
        fn cleanup_archives_sessions_from_the_database(temp_session_dir: TempSessionDir) {
            let now = Utc
                .with_ymd_and_hms(2026, 1, 10, 0, 0, 0)
                .single()
                .expect("timestamp should be valid");
            for session_id in ["locked-session", "available-session"] {
                let mut session = create_test_session(session_id);
                session.tmux_info = Some(TmuxInfo {
                    session_name: "test".to_string(),
                    window_name: "test".to_string(),
                    window_index: 0,
                    pane_id: "%99999".to_string(),
                    socket_path: None,
                });
                save_session_to(&temp_session_dir.sessions_path, &session)
                    .expect("save should succeed");
            }

            let changed = archive::cleanup_stale_sessions_in_with_archive_at(
                &temp_session_dir.sessions_path,
                mock_pane_always_dead,
                mock_cwd_always_exists,
                now,
                |_| Ok(()),
            )
            .expect("cleanup should succeed");

            let actual = (
                changed,
                load_session_from(&temp_session_dir.sessions_path, "locked-session")
                    .expect("load should succeed")
                    .map(|session| session.archived_at),
                load_session_from(&temp_session_dir.sessions_path, "available-session")
                    .expect("load should succeed")
                    .map(|session| session.archived_at),
            );
            assert_eq!(actual, (true, Some(Some(now)), Some(Some(now))));
        }

        #[rstest]
        fn keeps_session_with_alive_pane(temp_session_dir: TempSessionDir) {
            let session_id = "alive-pane-test";
            let path = session_record_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_record_in should succeed");

            let mut session = create_test_session(session_id);
            session.tmux_info = Some(TmuxInfo {
                session_name: "test".to_string(),
                window_name: "test".to_string(),
                window_index: 0,
                pane_id: "%1".to_string(),
                socket_path: None,
            });
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");

            cleanup_stale_sessions_in(
                &temp_session_dir.sessions_path,
                mock_pane_always_alive,
                mock_cwd_always_exists,
            )
            .expect("cleanup should succeed");

            assert!(path.is_stored(), "session with alive pane should be kept");
        }

        #[rstest]
        fn keeps_session_without_tmux_info(temp_session_dir: TempSessionDir) {
            let session_id = "no-tmux-test";
            let path = session_record_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_record_in should succeed");

            let mut session = create_test_session(session_id);
            session.tmux_info = None;
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");

            // Even with mock that treats all panes as dead, sessions without
            // tmux_info should be kept.
            cleanup_stale_sessions_in(
                &temp_session_dir.sessions_path,
                mock_pane_always_dead,
                mock_cwd_always_exists,
            )
            .expect("cleanup should succeed");

            assert!(path.is_stored(), "session without tmux_info should be kept");
        }

        #[rstest]
        fn archives_session_without_creating_a_lock_file(temp_session_dir: TempSessionDir) {
            let session_id = "with-lock-test";
            let path = session_record_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_record_in should succeed");

            let mut session = create_test_session(session_id);
            session.tmux_info = Some(TmuxInfo {
                session_name: "test".to_string(),
                window_name: "test".to_string(),
                window_index: 0,
                pane_id: "%99999".to_string(),
                socket_path: None,
            });
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");

            let lock_path = path.legacy_json_lock_path();
            assert!(path.is_stored(), "session record should exist");

            let archived_at = Utc
                .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("timestamp should be valid");
            let changed = archive::cleanup_stale_sessions_in_with_archive_at(
                &temp_session_dir.sessions_path,
                mock_pane_always_dead,
                mock_cwd_always_exists,
                archived_at,
                |_| Ok(()),
            )
            .expect("cleanup should succeed");

            let stored_archived_at = load_session_from(&temp_session_dir.sessions_path, session_id)
                .expect("load should succeed")
                .map(|session| session.archived_at);
            assert_eq!(
                (
                    changed,
                    path.is_stored(),
                    lock_path.exists(),
                    stored_archived_at
                ),
                (true, true, false, Some(Some(archived_at))),
            );
        }

        #[rstest]
        #[case::running(SessionStatus::Running)]
        #[case::waiting(SessionStatus::WaitingInput)]
        #[case::stopped(SessionStatus::Stopped)]
        #[case::paused(SessionStatus::Paused)]
        fn archives_session_when_cwd_is_gone(
            temp_session_dir: TempSessionDir,
            #[case] status: SessionStatus,
        ) {
            let session_id = "orphan-test";
            let path = session_record_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_record_in should succeed");

            let mut session = create_test_session(session_id);
            session.status = status;
            // tmux_info is None here to also cover the straggler-hook case:
            // a Notification fired after worktree cleanup recreates the file with
            // no pane info, so `stale_pane` alone would never catch it.
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");
            assert!(
                path.is_stored(),
                "session record should exist before cleanup"
            );

            let archived_at = Utc
                .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("timestamp should be valid");
            let changed = archive::cleanup_stale_sessions_in_with_archive_at(
                &temp_session_dir.sessions_path,
                mock_pane_always_alive,
                mock_cwd_always_missing,
                archived_at,
                |_| Ok(()),
            )
            .expect("cleanup should succeed");

            let stored_archived_at = load_session_from(&temp_session_dir.sessions_path, session_id)
                .expect("load should succeed")
                .map(|session| session.archived_at);
            assert_eq!(
                (changed, path.is_stored(), stored_archived_at),
                (true, true, Some(Some(archived_at))),
            );
        }

        #[rstest]
        #[case::running(SessionStatus::Running)]
        #[case::waiting(SessionStatus::WaitingInput)]
        #[case::stopped(SessionStatus::Stopped)]
        #[case::paused(SessionStatus::Paused)]
        fn keeps_session_when_cwd_still_exists(
            temp_session_dir: TempSessionDir,
            #[case] status: SessionStatus,
        ) {
            let session_id = "live-test";
            let path = session_record_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_record_in should succeed");

            let mut session = create_test_session(session_id);
            session.status = status;
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");

            cleanup_stale_sessions_in(
                &temp_session_dir.sessions_path,
                mock_pane_always_dead,
                mock_cwd_always_exists,
            )
            .expect("cleanup should succeed");

            assert!(
                path.is_stored(),
                "session should be kept while its cwd exists"
            );
        }

        #[rstest]
        fn keeps_ended_session_when_cwd_is_gone_but_not_expired(temp_session_dir: TempSessionDir) {
            let session_id = "ended-not-expired";
            let path = session_record_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_record_in should succeed");

            let mut session = create_test_session(session_id);
            session.status = SessionStatus::Ended;
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");

            cleanup_stale_sessions_in(
                &temp_session_dir.sessions_path,
                mock_pane_always_dead,
                mock_cwd_always_missing,
            )
            .expect("cleanup should succeed");

            assert!(
                path.is_stored(),
                "ended session should rely on retention, not cwd existence"
            );
        }

        #[rstest]
        #[case::ended(SessionStatus::Ended, false)]
        #[case::paused(SessionStatus::Paused, false)]
        #[case::archived(SessionStatus::Stopped, true)]
        fn removes_expired_sessions_without_lock_files(
            temp_session_dir: TempSessionDir,
            #[case] status: SessionStatus,
            #[case] archived: bool,
        ) {
            let session_id = "expired-session";
            let path = session_record_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_record_in should succeed");
            let lock_path = path.legacy_json_lock_path();
            let now = Utc
                .with_ymd_and_hms(2026, 1, 10, 0, 0, 0)
                .single()
                .expect("timestamp should be valid");
            let mut session = create_test_session(session_id);
            session.status = status;
            if archived {
                session.archived_at = Some(now - TimeDelta::days(8));
            } else {
                session.updated_at = now - TimeDelta::days(8);
            }
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");

            let removed = archive::cleanup_stale_sessions_in_with_archive_at(
                &temp_session_dir.sessions_path,
                mock_pane_always_alive,
                mock_cwd_always_exists,
                now,
                |_| Ok(()),
            )
            .expect("cleanup should succeed");

            assert_eq!(
                (removed, path.is_stored(), lock_path.exists()),
                (true, false, false),
            );
        }

        #[rstest]
        fn retains_archived_session_until_its_archive_retention_expires(
            temp_session_dir: TempSessionDir,
        ) {
            let session_id = "recently-archived";
            let path = session_record_in(&temp_session_dir.sessions_path, session_id)
                .expect("session_record_in should succeed");
            let now = Utc
                .with_ymd_and_hms(2026, 1, 10, 0, 0, 0)
                .single()
                .expect("timestamp should be valid");
            let archived_at = now - TimeDelta::days(3);
            let mut session = create_test_session(session_id);
            session.archived_at = Some(archived_at);
            save_session_to(&temp_session_dir.sessions_path, &session)
                .expect("save should succeed");

            let changed = archive::cleanup_stale_sessions_in_with_archive_at(
                &temp_session_dir.sessions_path,
                mock_pane_always_dead,
                mock_cwd_always_missing,
                now,
                |_| Ok(()),
            )
            .expect("cleanup should succeed");
            let stored_archived_at = load_session_from(&temp_session_dir.sessions_path, session_id)
                .expect("load should succeed")
                .map(|session| session.archived_at);

            assert_eq!(
                (changed, path.is_stored(), stored_archived_at),
                (false, true, Some(Some(archived_at))),
            );
        }

        #[rstest]
        fn cleanup_uses_recorded_sockets_and_keeps_records_when_a_server_is_unavailable(
            temp_session_dir: TempSessionDir,
        ) {
            let records = [
                ("default-live", "%731", None),
                ("default-stale", "%732", None),
                ("private-live", "%733", Some("/tmp/armyknife-test/private")),
                (
                    "unavailable-server",
                    "%734",
                    Some("/tmp/armyknife-test/unavailable"),
                ),
            ];
            let mut paths = HashMap::new();

            for (session_id, pane_id, socket_path) in records {
                let mut session = create_test_session(session_id);
                session.tmux_info = Some(TmuxInfo {
                    session_name: "fixture-session".to_string(),
                    window_name: "fixture-window".to_string(),
                    window_index: 0,
                    pane_id: pane_id.to_string(),
                    socket_path: socket_path.map(str::to_owned),
                });
                let path = session_record_in(&temp_session_dir.sessions_path, session_id)
                    .expect("session path should be valid");
                save_session_to(&temp_session_dir.sessions_path, &session)
                    .expect("session should save");
                paths.insert(session_id, path);
            }

            let query_counts = std::cell::RefCell::new(HashMap::<Option<String>, usize>::new());
            let removed = cleanup_stale_sessions_by_socket(
                &temp_session_dir.sessions_path,
                |socket_path| {
                    let socket_path = socket_path.map(str::to_owned);
                    *query_counts
                        .borrow_mut()
                        .entry(socket_path.clone())
                        .or_default() += 1;
                    match socket_path.as_deref() {
                        None => Ok(HashSet::from(["%731".to_string()])),
                        Some("/tmp/armyknife-test/private") => {
                            Ok(HashSet::from(["%733".to_string()]))
                        }
                        Some("/tmp/armyknife-test/unavailable") => Err(tmux::TmuxError::Internal(
                            "fixture server unavailable".to_string(),
                        )),
                        Some(_) => Err(tmux::TmuxError::Internal("unexpected socket".to_string())),
                    }
                },
                mock_cwd_always_exists,
            )
            .expect("cleanup should tolerate a server query failure");

            assert_eq!(
                (
                    removed,
                    paths["default-live"].is_stored(),
                    paths["default-stale"].is_stored(),
                    paths["private-live"].is_stored(),
                    paths["unavailable-server"].is_stored(),
                    query_counts.into_inner(),
                ),
                (
                    true,
                    true,
                    true,
                    true,
                    true,
                    HashMap::from([
                        (None, 1),
                        (Some("/tmp/armyknife-test/private".to_string()), 1),
                        (Some("/tmp/armyknife-test/unavailable".to_string()), 1),
                    ]),
                ),
            );
        }
    }

    mod mark_session_read_tests {
        use super::*;
        use rstest::rstest;

        fn make_session(status: SessionStatus, read_at: Option<DateTime<Utc>>) -> Session {
            let mut s = create_test_session("mark-target");
            s.status = status;
            s.read_at = read_at;
            s
        }

        #[rstest]
        // unread Stopped → marked read.
        #[case::unread_stopped_marked(SessionStatus::Stopped, None, true)]
        // already-read Stopped → no-op (preserves the existing timestamp).
        #[case::read_stopped_noop(
            SessionStatus::Stopped,
            Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
            false,
        )]
        // Running session must not gain read_at.
        #[case::running_noop(SessionStatus::Running, None, false)]
        // Ended session is not "Stopped" and must not gain read_at.
        #[case::ended_noop(SessionStatus::Ended, None, false)]
        fn marks_only_unread_stopped(
            temp_session_dir: TempSessionDir,
            #[case] status: SessionStatus,
            #[case] initial: Option<DateTime<Utc>>,
            #[case] now_should_be_written: bool,
        ) {
            let session = make_session(status, initial);
            save_session_to(&temp_session_dir.sessions_path, &session).expect("save");

            let now = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
            mark_session_read_in(&temp_session_dir.sessions_path, "mark-target", now)
                .expect("mark-read should succeed");

            let reloaded = load_session_from(&temp_session_dir.sessions_path, "mark-target")
                .expect("load")
                .expect("session exists");
            let expected = if now_should_be_written {
                Some(now)
            } else {
                initial
            };
            assert_eq!(reloaded.read_at, expected);
        }

        #[rstest]
        fn missing_session_record_is_noop(temp_session_dir: TempSessionDir) {
            let now = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
            mark_session_read_in(&temp_session_dir.sessions_path, "ghost", now)
                .expect("missing session should be ok");
        }
    }

    mod update_session_label_tests {
        use super::*;
        use rstest::rstest;

        fn make_session(label: Option<&str>) -> Session {
            let mut s = create_test_session("label-target");
            s.label = label.map(str::to_string);
            s
        }

        #[rstest]
        #[case::sets_label_on_none(None, Some("New Title"), Some("New Title"))]
        #[case::overwrites_existing_label(Some("Old Title"), Some("New Title"), Some("New Title"))]
        #[case::clears_via_none(Some("Old Title"), None, None)]
        fn overwrites_label(
            temp_session_dir: TempSessionDir,
            #[case] initial: Option<&str>,
            #[case] update: Option<&str>,
            #[case] expected: Option<&str>,
        ) {
            let session = make_session(initial);
            save_session_to(&temp_session_dir.sessions_path, &session).expect("save");

            update_session_label_in(
                &temp_session_dir.sessions_path,
                "label-target",
                update.map(str::to_string),
            )
            .expect("update should succeed");

            let reloaded = load_session_from(&temp_session_dir.sessions_path, "label-target")
                .expect("load")
                .expect("session exists");
            assert_eq!(reloaded.label, expected.map(str::to_string));
        }

        #[rstest]
        fn missing_session_record_is_noop(temp_session_dir: TempSessionDir) {
            update_session_label_in(
                &temp_session_dir.sessions_path,
                "ghost",
                Some("New Title".to_string()),
            )
            .expect("missing session should be ok");
        }
    }

    mod update_session_label_if_unchanged_tests {
        use super::*;
        use rstest::rstest;

        fn make_session(label: Option<&str>) -> Session {
            let mut s = create_test_session("label-target");
            s.label = label.map(str::to_string);
            s
        }

        #[rstest]
        #[case::none_to_none_matches(None, None, Some("New Title"))]
        #[case::some_to_some_matches(Some("Old Title"), Some("Old Title"), Some("New Title"))]
        fn applies_when_expected_matches_current(
            temp_session_dir: TempSessionDir,
            #[case] initial: Option<&str>,
            #[case] expected_current: Option<&str>,
            #[case] new_label: Option<&str>,
        ) {
            let session = make_session(initial);
            save_session_to(&temp_session_dir.sessions_path, &session).expect("save");

            let applied = update_session_label_if_unchanged_in(
                &temp_session_dir.sessions_path,
                "label-target",
                expected_current,
                new_label.map(str::to_string),
            )
            .expect("update should succeed");

            let reloaded = load_session_from(&temp_session_dir.sessions_path, "label-target")
                .expect("load")
                .expect("session exists");
            assert_eq!(
                (applied, reloaded.label),
                (true, new_label.map(str::to_string))
            );
        }

        #[rstest]
        #[case::none_to_some_mismatches(None, Some("Something Else"))]
        #[case::some_to_none_mismatches(Some("Old Title"), None)]
        #[case::some_to_different_some_mismatches(Some("Old Title"), Some("Different"))]
        fn does_not_apply_when_current_label_changed(
            temp_session_dir: TempSessionDir,
            #[case] initial: Option<&str>,
            #[case] expected_current: Option<&str>,
        ) {
            let session = make_session(initial);
            save_session_to(&temp_session_dir.sessions_path, &session).expect("save");

            let applied = update_session_label_if_unchanged_in(
                &temp_session_dir.sessions_path,
                "label-target",
                expected_current,
                Some("New Title".to_string()),
            )
            .expect("update should succeed");

            let reloaded = load_session_from(&temp_session_dir.sessions_path, "label-target")
                .expect("load")
                .expect("session exists");
            assert_eq!(
                (applied, reloaded.label),
                (false, initial.map(str::to_string))
            );
        }

        #[rstest]
        fn missing_session_record_returns_false_without_error(temp_session_dir: TempSessionDir) {
            let applied = update_session_label_if_unchanged_in(
                &temp_session_dir.sessions_path,
                "ghost",
                None,
                Some("New Title".to_string()),
            )
            .expect("missing session should be ok");

            assert!(!applied);
        }
    }

    mod update_session_tmux_pane_id_tests {
        use super::*;
        use crate::commands::agent::types::TmuxInfo;
        use rstest::rstest;

        fn make_session(tmux_info: Option<TmuxInfo>) -> Session {
            let mut s = create_test_session("pane-target");
            s.tmux_info = tmux_info;
            s
        }

        fn tmux_info(pane_id: &str) -> TmuxInfo {
            TmuxInfo {
                session_name: "main".to_string(),
                window_name: "dev".to_string(),
                window_index: 0,
                pane_id: pane_id.to_string(),
                socket_path: Some("/tmp/armyknife-test/default".to_string()),
            }
        }

        #[rstest]
        #[case::new_pane_id("%1", "%42")]
        #[case::same_pane_id("%42", "%42")]
        fn updates_pane_id_and_clears_archive(
            temp_session_dir: TempSessionDir,
            #[case] existing_pane_id: &str,
            #[case] restored_pane_id: &str,
        ) {
            let mut session = make_session(Some(tmux_info(existing_pane_id)));
            let archived_at = Utc
                .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("timestamp should be valid");
            session.archived_at = Some(archived_at);
            save_session_to(&temp_session_dir.sessions_path, &session).expect("save");

            update_session_tmux_pane_id_in(
                &temp_session_dir.sessions_path,
                "pane-target",
                restored_pane_id,
            )
            .expect("update should succeed");

            let reloaded = load_session_from(&temp_session_dir.sessions_path, "pane-target")
                .expect("load")
                .expect("session exists");
            assert_eq!(
                (reloaded.tmux_info, reloaded.archived_at),
                (
                    Some(TmuxInfo {
                        session_name: "main".to_string(),
                        window_name: "dev".to_string(),
                        window_index: 0,
                        pane_id: restored_pane_id.to_string(),
                        socket_path: Some("/tmp/armyknife-test/default".to_string()),
                    }),
                    None,
                ),
            );
        }

        #[rstest]
        fn clears_archive_even_when_tmux_info_is_missing(temp_session_dir: TempSessionDir) {
            let mut session = make_session(None);
            session.archived_at = Some(
                Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                    .single()
                    .expect("timestamp should be valid"),
            );
            save_session_to(&temp_session_dir.sessions_path, &session).expect("save");

            update_session_tmux_pane_id_in(&temp_session_dir.sessions_path, "pane-target", "%42")
                .expect("update should succeed");

            let reloaded = load_session_from(&temp_session_dir.sessions_path, "pane-target")
                .expect("load")
                .expect("session exists");
            assert_eq!((reloaded.tmux_info, reloaded.archived_at), (None, None));
        }

        #[rstest]
        fn missing_session_record_is_noop(temp_session_dir: TempSessionDir) {
            update_session_tmux_pane_id_in(&temp_session_dir.sessions_path, "ghost", "%42")
                .expect("missing session should be ok");
        }
    }

    mod update_session_last_message_if_unchanged_tests {
        use super::*;
        use rstest::rstest;

        fn make_session(last_message: Option<&str>) -> Session {
            let mut s = create_test_session("message-target");
            s.last_message = last_message.map(str::to_string);
            s
        }

        #[rstest]
        fn applies_when_expected_matches_current(temp_session_dir: TempSessionDir) {
            let session = make_session(Some("old message"));
            save_session_to(&temp_session_dir.sessions_path, &session).expect("save");

            update_session_last_message_if_unchanged_in(
                &temp_session_dir.sessions_path,
                "message-target",
                Some("old message"),
                Some("new message".to_string()),
            )
            .expect("update should succeed");

            let reloaded = load_session_from(&temp_session_dir.sessions_path, "message-target")
                .expect("load")
                .expect("session exists");
            assert_eq!(reloaded.last_message, Some("new message".to_string()));
        }

        #[rstest]
        fn does_not_apply_when_current_message_changed(temp_session_dir: TempSessionDir) {
            // Simulates a slower sibling hook event whose transcript read
            // (based on the stale "old message" snapshot) finishes after a
            // faster event already saved "message from newer event".
            let session = make_session(Some("message from newer event"));
            save_session_to(&temp_session_dir.sessions_path, &session).expect("save");

            update_session_last_message_if_unchanged_in(
                &temp_session_dir.sessions_path,
                "message-target",
                Some("old message"),
                Some("message from stale read".to_string()),
            )
            .expect("update should succeed");

            let reloaded = load_session_from(&temp_session_dir.sessions_path, "message-target")
                .expect("load")
                .expect("session exists");
            assert_eq!(
                reloaded.last_message,
                Some("message from newer event".to_string())
            );
        }

        #[rstest]
        fn missing_session_record_is_noop(temp_session_dir: TempSessionDir) {
            update_session_last_message_if_unchanged_in(
                &temp_session_dir.sessions_path,
                "ghost",
                None,
                Some("new message".to_string()),
            )
            .expect("missing session should be ok");
        }
    }

    mod corrupted_file_recovery_tests {
        use super::*;
        use rstest::rstest;

        #[test]
        fn invalid_legacy_json_does_not_require_the_writer_lock_for_reads() {
            let temp_session_dir = temp_session_dir();
            let legacy_json = temp_session_dir.sessions_path.join("invalid.json");
            fs::write(&legacy_json, "{").expect("invalid legacy JSON should be written");
            let mut connection = database::open_connection(&temp_session_dir.sessions_path)
                .expect("session database should open");
            let writer = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .expect("writer transaction should start");

            let loaded = load_session_from(&temp_session_dir.sessions_path, "not-present")
                .expect("read should not wait for a writer transaction");

            assert_eq!(
                (
                    loaded.is_none(),
                    legacy_json.with_extension("json.lock").exists()
                ),
                (true, false)
            );
            drop(writer);
        }

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
            let path = session_record_in(&temp_session_dir.sessions_path, &session_id)
                .expect("session_record_in should succeed");

            fs::write(path.legacy_json_path(), content).expect("write should succeed");

            let result = load_session_from(&temp_session_dir.sessions_path, &session_id)
                .expect("load should not error");
            assert!(result.is_none(), "{} session should return None", prefix);
            // Invalid legacy JSON is preserved for inspection.
            assert!(
                path.legacy_json_path().exists(),
                "{} JSON should still exist",
                prefix
            );
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
                archived_at: None,
                pr_context: None,
                session_id: id.to_string(),
                work_type: None,
                work_type_pinned: false,
                vibe_enabled: false,
                launched_with_vibe: false,
                is_delegated: false,
                crit_urls: Vec::new(),
                pending_human_review_ids: Default::default(),
                cwd: PathBuf::from("/tmp/test"),
                transcript_path: None,
                tty: None,
                tmux_info: None,
                status: SessionStatus::Running,
                created_at,
                updated_at,
                last_message: None,
                current_tool: None,
                label: None,
                ancestor_session_ids: Vec::new(),
                pending_bg_task_ids: std::collections::BTreeSet::new(),
                pending_agent_task_ids: std::collections::BTreeSet::new(),
                pending_permission_agent_ids: std::collections::BTreeSet::new(),
                pending_permission_request_ids: Default::default(),
                read_at: None,
                sweep_signaled: false,
                stop_blocked_by_armyknife: false,
                agent_status: None,
                agent_status_clear_pending: false,
                engine: Engine::Claude,
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

    mod last_selected_session_tests {
        use super::*;
        use rstest::rstest;

        #[rstest]
        fn save_and_load_round_trip(temp_session_dir: TempSessionDir) {
            let file_path = temp_session_dir.sessions_path.join("last_selected_session");
            let session_id = "test-session-123";

            save_last_selected_session_to(&file_path, session_id).expect("save should succeed");

            let loaded = load_last_selected_session_from(&file_path).expect("load should succeed");

            assert_eq!(loaded, Some(session_id.to_string()));
        }

        #[rstest]
        fn load_returns_none_for_nonexistent_file(temp_session_dir: TempSessionDir) {
            let file_path = temp_session_dir.sessions_path.join("nonexistent");

            assert!(!file_path.exists());

            let result = load_last_selected_session_from(&file_path).expect("load should succeed");

            assert!(result.is_none());
        }

        #[rstest]
        fn load_returns_none_for_empty_file(temp_session_dir: TempSessionDir) {
            let file_path = temp_session_dir.sessions_path.join("empty_session");

            // Write empty content directly to test the load function behavior
            fs::write(&file_path, "").expect("write should succeed");

            let result = load_last_selected_session_from(&file_path).expect("load should succeed");

            assert!(result.is_none());
        }

        #[rstest]
        fn load_trims_whitespace(temp_session_dir: TempSessionDir) {
            let file_path = temp_session_dir.sessions_path.join("whitespace_session");

            // Write with trailing newline directly to test the load function behavior
            fs::write(&file_path, "test-session\n").expect("write should succeed");

            let loaded = load_last_selected_session_from(&file_path).expect("load should succeed");

            assert_eq!(loaded, Some("test-session".to_string()));
        }

        #[rstest]
        fn save_creates_parent_directories(temp_session_dir: TempSessionDir) {
            let nested_path = temp_session_dir
                .sessions_path
                .join("nested")
                .join("dir")
                .join("last_selected_session");
            let session_id = "nested-test";

            save_last_selected_session_to(&nested_path, session_id).expect("save should succeed");

            assert!(nested_path.exists());
            let loaded =
                load_last_selected_session_from(&nested_path).expect("load should succeed");
            assert_eq!(loaded, Some(session_id.to_string()));
        }

        #[rstest]
        fn save_overwrites_existing_file(temp_session_dir: TempSessionDir) {
            let file_path = temp_session_dir.sessions_path.join("overwrite_test");

            save_last_selected_session_to(&file_path, "first").expect("save should succeed");
            save_last_selected_session_to(&file_path, "second").expect("save should succeed");

            let loaded = load_last_selected_session_from(&file_path).expect("load should succeed");
            assert_eq!(loaded, Some("second".to_string()));
        }
    }
}
