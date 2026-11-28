use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, TimeDelta, Utc};

use crate::commands::agent::types::{Engine, Session, SessionStatus, TmuxInfo};

#[derive(Clone, Copy)]
pub(crate) enum ArchiveLogContext {
    Store,
    WorktreeCleanup,
}

/// Archives a Codex thread before deleting its session row.
/// Returns false when an incomplete todo keeps the row available or it was already absent.
pub(crate) fn delete_session_with_archive(session_id: &str) -> Result<bool> {
    let sessions_dir = super::sessions_dir()?;
    let storage_root = super::agent_file_storage_root();
    delete_session_with_archive_in(
        &sessions_dir,
        session_id,
        storage_root.as_deref(),
        crate::commands::agent::codex_steer::archive_thread,
    )
}

fn delete_session_with_archive_in(
    sessions_dir: &Path,
    session_id: &str,
    storage_root: Option<&Path>,
    mut archive_codex_thread: impl FnMut(&str) -> anyhow::Result<()>,
) -> Result<bool> {
    if !sessions_dir.exists() {
        return Ok(false);
    }

    // Keep todo state stable until deletion so the row is neither deleted unarchived nor kept after archiving.
    let _todo_resolution_lock = super::lock_agent_todo_resolution(sessions_dir, session_id)?;
    let session_lock = super::lock_session_for_update(sessions_dir, session_id)?;
    let session = session_lock.load_skipping_undecodable()?;
    let should_archive = match session.as_ref() {
        Some(session) if session.engine == Engine::Codex => !session_lock.has_incomplete_todos()?,
        _ => false,
    };
    drop(session_lock);

    if should_archive && let Some(session) = session {
        archive_codex_thread_before_delete_with(
            &session,
            &mut archive_codex_thread,
            ArchiveLogContext::Store,
        );
    }

    super::delete_session_from_with_storage(sessions_dir, session_id, storage_root)
}

pub(crate) fn archive_codex_thread_before_delete_with(
    session: &Session,
    archive_codex_thread: &mut impl FnMut(&str) -> anyhow::Result<()>,
    log_context: ArchiveLogContext,
) {
    if session.engine != Engine::Codex {
        return;
    }

    if let Err(error) = archive_codex_thread(&session.session_id) {
        match log_context {
            ArchiveLogContext::Store => tracing::warn!(
                target: "armyknife::commands::agent::store",
                event = "store.codex_archive.err",
                session = %session.session_id,
                msg = format!("failed to archive Codex thread: {error:#}"),
            ),
            ArchiveLogContext::WorktreeCleanup => tracing::warn!(
                target: "armyknife::shared::cleanup",
                event = "cleanup.codex_archive.err",
                session = %session.session_id,
                msg = format!("failed to archive Codex thread: {error:#}"),
            ),
        }
    }
}

#[cfg(test)]
pub(super) fn cleanup_stale_sessions_in_with_archive_at<F, G, A>(
    dir: &Path,
    is_pane_alive: F,
    cwd_exists: G,
    now: DateTime<Utc>,
    archive_codex_thread: A,
) -> Result<bool>
where
    F: Fn(&TmuxInfo) -> Option<bool>,
    G: Fn(&Path) -> bool,
    A: FnMut(&str) -> anyhow::Result<()>,
{
    cleanup_stale_sessions_in_with_archive_at_and_storage(
        dir,
        None,
        is_pane_alive,
        cwd_exists,
        now,
        archive_codex_thread,
    )
}

pub(super) fn cleanup_stale_sessions_in_with_archive_at_and_storage<F, G, A>(
    dir: &Path,
    storage_root: Option<&Path>,
    is_pane_alive: F,
    cwd_exists: G,
    now: DateTime<Utc>,
    archive_codex_thread: A,
) -> Result<bool>
where
    F: Fn(&TmuxInfo) -> Option<bool>,
    G: Fn(&Path) -> bool,
    A: FnMut(&str) -> anyhow::Result<()>,
{
    cleanup_stale_sessions_in_with_archive_at_storage_and_crit_cleanup(
        dir,
        storage_root,
        is_pane_alive,
        cwd_exists,
        now,
        archive_codex_thread,
        crate::commands::agent::crit::stop_expired_session_daemons,
    )
}

fn cleanup_stale_sessions_in_with_archive_at_storage_and_crit_cleanup<F, G, A, C>(
    dir: &Path,
    storage_root: Option<&Path>,
    is_pane_alive: F,
    cwd_exists: G,
    now: DateTime<Utc>,
    mut archive_codex_thread: A,
    mut stop_expired_crit_daemons: C,
) -> Result<bool>
where
    F: Fn(&TmuxInfo) -> Option<bool>,
    G: Fn(&Path) -> bool,
    A: FnMut(&str) -> anyhow::Result<()>,
    C: FnMut(&Session, &Path) -> anyhow::Result<()>,
{
    if !dir.exists() {
        return Ok(false);
    }

    let retention = TimeDelta::days(super::ENDED_SESSION_RETENTION_DAYS);
    let sessions = super::database::all_sessions(dir)?;
    let vibe_inheritance_ancestors = vibe_inheritance_ancestors(&sessions, now, retention);
    let mut changed_any = false;

    for session in &sessions {
        let session_id = session.session_id.as_str();
        if is_collectable(session, now, retention) {
            // Descendants resolve inherited vibe mode by reading this record.
            let keeps_vibe_inheritance =
                session.vibe_enabled && vibe_inheritance_ancestors.contains(session_id);
            if !keeps_vibe_inheritance {
                changed_any |= collect_expired_session(
                    dir,
                    storage_root,
                    session_id,
                    now,
                    retention,
                    &mut archive_codex_thread,
                    &mut stop_expired_crit_daemons,
                );
            }
            continue;
        }

        let stale_pane = session.status != SessionStatus::Ended
            && session
                .tmux_info
                .as_ref()
                .is_some_and(|info| is_pane_alive(info) == Some(false));
        let orphaned = session.status != SessionStatus::Ended && !cwd_exists(&session.cwd);

        if (stale_pane || orphaned)
            && archive_stale_session(dir, session_id, session, stale_pane, now, &cwd_exists)
        {
            changed_any = true;
            crate::shared::cleanup::withdraw_ended_session_cards(dir, session_id);
        }
    }

    Ok(changed_any)
}

fn vibe_inheritance_ancestors(
    sessions: &[Session],
    now: DateTime<Utc>,
    retention: TimeDelta,
) -> HashSet<&str> {
    sessions
        .iter()
        .filter(|session| session.launched_with_vibe && !is_collectable(session, now, retention))
        .flat_map(|session| session.ancestor_session_ids.iter().map(String::as_str))
        .collect()
}

fn collect_expired_session<A, C>(
    dir: &Path,
    storage_root: Option<&Path>,
    session_id: &str,
    now: DateTime<Utc>,
    retention: TimeDelta,
    archive_codex_thread: &mut A,
    stop_expired_crit_daemons: &mut C,
) -> bool
where
    A: FnMut(&str) -> anyhow::Result<()>,
    C: FnMut(&Session, &Path) -> anyhow::Result<()>,
{
    // Keep todo state stable so an incomplete todo cannot keep a session after its thread is archived.
    let _todo_resolution_lock = match super::lock_agent_todo_resolution(dir, session_id) {
        Ok(lock) => lock,
        Err(error) => {
            warn_cleanup_entry_failure(session_id, "lock_todo_resolution", &error);
            return false;
        }
    };
    let Some((session_lock, current)) =
        lock_and_load_session_for_cleanup(dir, session_id, "collection")
    else {
        return false;
    };
    if !is_collectable(&current, now, retention) {
        return false;
    }

    let session_lock = if current.engine == Engine::Codex {
        match session_lock.has_incomplete_todos() {
            Ok(true) => return false,
            Ok(false) => {}
            Err(error) => {
                warn_cleanup_entry_failure(session_id, "check_incomplete_todos", &error);
                return false;
            }
        }
        let snapshot = match super::serialize_session(&current) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                warn_cleanup_entry_failure(session_id, "snapshot_before_archive", &error);
                return false;
            }
        };
        drop(session_lock);
        archive_codex_thread_before_delete_with(
            &current,
            archive_codex_thread,
            ArchiveLogContext::Store,
        );
        let Some((session_lock, current)) =
            lock_and_load_session_for_cleanup(dir, session_id, "collection_after_archive")
        else {
            return false;
        };
        let current_snapshot = match super::serialize_session(&current) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                warn_cleanup_entry_failure(session_id, "snapshot_after_archive", &error);
                return false;
            }
        };
        if current_snapshot != snapshot || !is_collectable(&current, now, retention) {
            return false;
        }
        session_lock
    } else {
        session_lock
    };

    let removed = match session_lock.delete() {
        Ok(removed) => removed,
        Err(error) => {
            warn_cleanup_entry_failure(session_id, "delete", &error);
            false
        }
    };
    drop(session_lock);
    if removed
        && let Some(storage_root) = storage_root
        && let Err(error) =
            crate::commands::agent::file::remove_session_storage(storage_root, session_id)
    {
        warn_cleanup_entry_failure(session_id, "delete_attachment_files", &error);
    }
    if removed && let Err(error) = stop_expired_crit_daemons(&current, dir) {
        warn_cleanup_entry_failure(session_id, "stop_expired_crit_daemons", &error);
    }
    removed
}

fn archive_stale_session<G>(
    dir: &Path,
    session_id: &str,
    session: &Session,
    stale_pane: bool,
    now: DateTime<Utc>,
    cwd_exists: &G,
) -> bool
where
    G: Fn(&Path) -> bool,
{
    let Some((session_lock, mut current)) =
        lock_and_load_session_for_cleanup(dir, session_id, "archive")
    else {
        return false;
    };
    if current.archived_at.is_some()
        || current.updated_at != session.updated_at
        || (stale_pane
            && (current.status != session.status || current.tmux_info != session.tmux_info))
    {
        return false;
    }

    let still_orphaned = current.status != SessionStatus::Ended && !cwd_exists(&current.cwd);
    if !still_orphaned && !stale_pane {
        return false;
    }

    current.archived_at = Some(now);
    if let Err(error) = session_lock.save(&current) {
        warn_cleanup_entry_failure(session_id, "save_archive_state", &error);
        return false;
    }
    true
}

fn lock_and_load_session_for_cleanup(
    dir: &Path,
    session_id: &str,
    operation: &str,
) -> Option<(super::SessionLock, Session)> {
    let session_lock = match super::lock_session_for_update(dir, session_id) {
        Ok(lock) => lock,
        Err(error) => {
            warn_cleanup_entry_failure(session_id, &format!("lock_for_{operation}"), &error);
            return None;
        }
    };
    let session = match session_lock.load() {
        Ok(Some(session)) => session,
        Ok(None) => return None,
        Err(error) => {
            warn_cleanup_entry_failure(session_id, &format!("reload_for_{operation}"), &error);
            return None;
        }
    };

    Some((session_lock, session))
}

fn warn_cleanup_entry_failure(session_id: &str, operation: &str, error: &impl std::fmt::Display) {
    tracing::warn!(
        target: "armyknife::commands::agent::store",
        event = "agent.store.cleanup_entry_failed",
        session = session_id,
        operation,
        error = %error,
    );
}

fn is_collectable(session: &Session, now: DateTime<Utc>, retention: TimeDelta) -> bool {
    if session.archived_at.is_some() {
        is_expired_archive(session, now, retention)
    } else {
        is_expired_ended(session, now, retention)
    }
}

fn is_expired_archive(session: &Session, now: DateTime<Utc>, retention: TimeDelta) -> bool {
    session
        .archived_at
        .as_ref()
        .is_some_and(|archived_at| now - *archived_at > retention)
}

fn is_expired_ended(session: &Session, now: DateTime<Utc>, retention: TimeDelta) -> bool {
    matches!(session.status, SessionStatus::Ended | SessionStatus::Paused)
        && now - session.updated_at > retention
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::test_support::{agent_card_state, session_end_cards};
    use crate::commands::agent::types::{SessionStatus, TmuxInfo};
    use chrono::Utc;
    use rstest::{fixture, rstest};
    use sha2::Digest;
    use std::cell::RefCell;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    struct TestSessionDir {
        temp_dir: TempDir,
        sessions_dir: PathBuf,
    }

    struct IncompleteTodoSession {
        test_session_dir: TestSessionDir,
        now: chrono::DateTime<Utc>,
        session_id: String,
        todo_seq: i64,
    }

    #[fixture]
    fn test_session_dir() -> TestSessionDir {
        let temp_dir = TempDir::new().expect("temp directory creation should succeed");
        let sessions_dir = temp_dir.path().join("sessions");
        fs::create_dir_all(&sessions_dir).expect("sessions directory creation should succeed");
        TestSessionDir {
            temp_dir,
            sessions_dir,
        }
    }

    #[fixture]
    fn incomplete_todo_session(test_session_dir: TestSessionDir) -> IncompleteTodoSession {
        use super::super::database::add_agent_todo;

        let now = Utc::now();
        let session_id = "session-with-todo".to_string();
        let mut session = make_session(&session_id, Engine::Codex);
        session.crit_urls = vec!["http://review.example.test:43120/review".to_string()];
        session.status = SessionStatus::Ended;
        session.updated_at = now - TimeDelta::days(super::super::ENDED_SESSION_RETENTION_DAYS + 1);
        super::super::save_session_to(&test_session_dir.sessions_dir, &session)
            .expect("session save should succeed");
        let todo_seq = add_agent_todo(
            &test_session_dir.sessions_dir,
            &session_id,
            "Finish task",
            None,
            false,
        )
        .expect("todo should be created");

        IncompleteTodoSession {
            test_session_dir,
            now,
            session_id,
            todo_seq,
        }
    }

    fn make_session(session_id: &str, engine: Engine) -> Session {
        Session {
            launch_id: None,
            archived_at: None,
            pr_context: None,
            created_pr_urls: Vec::new(),
            session_id: session_id.to_string(),
            crit_urls: Vec::new(),
            crit_not_waiting_urls: Default::default(),
            web_uis: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp/example-worktree"),
            transcript_path: None,
            tty: None,
            tmux_info: Some(TmuxInfo {
                session_name: "example".to_string(),
                window_name: "example".to_string(),
                window_index: 0,
                pane_id: "%1".to_string(),
                socket_path: None,
            }),
            status: SessionStatus::Stopped,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_prompted_at: None,
            last_message: None,
            current_tool: None,
            current_tool_name: None,
            label: None,
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine,
            model: None,
            effort: None,
        }
    }

    #[rstest]
    #[case::pane_gone(false, true, true)]
    #[case::cwd_gone(true, false, true)]
    #[case::live_session(true, true, false)]
    fn stale_session_cleanup_withdraws_unanswered_cards(
        test_session_dir: TestSessionDir,
        #[case] pane_is_alive: bool,
        #[case] cwd_exists: bool,
        #[case] is_archived: bool,
    ) {
        let TestSessionDir { sessions_dir, .. } = test_session_dir;
        let now = Utc::now();
        let session_id = "session-with-cards";
        let mut session = make_session(session_id, Engine::Claude);
        session.updated_at = now;
        super::super::save_session_to(&sessions_dir, &session)
            .expect("session save should succeed");
        session_end_cards().add_to(&sessions_dir, session_id);

        cleanup_stale_sessions_in_with_archive_at(
            &sessions_dir,
            |_| Some(pane_is_alive),
            |_| cwd_exists,
            now,
            |_| Ok(()),
        )
        .expect("stale session cleanup should succeed");

        let archived_at = super::super::load_session_from(&sessions_dir, session_id)
            .expect("session lookup should succeed")
            .expect("session should remain stored")
            .archived_at;
        let withdrawn_comment =
            is_archived.then_some("Withdrawn because the session ended.".to_string());
        let withdrawal_reason = is_archived.then_some(super::super::WithdrawalReason::SessionEnded);
        assert_eq!(
            (
                archived_at,
                agent_card_state(&sessions_dir, session_id).expect("agent cards should load"),
            ),
            (
                is_archived.then_some(now),
                (
                    vec![
                        (1, Some(super::super::TodoResolution::Completed), None, None),
                        (
                            2,
                            is_archived.then_some(super::super::TodoResolution::Withdrawn),
                            withdrawal_reason,
                            withdrawn_comment.clone(),
                        ),
                    ],
                    vec![
                        (1, None, true, None, None),
                        (
                            2,
                            is_archived.then_some(super::super::ProposalResolution::Withdrawn),
                            is_archived,
                            withdrawal_reason,
                            withdrawn_comment,
                        ),
                    ],
                ),
            ),
        );
    }

    #[rstest]
    #[case::claude(Engine::Claude, Vec::new())]
    #[case::codex(Engine::Codex, vec![("session-to-delete".to_string(), true)])]
    fn archives_codex_before_deleting_session(
        test_session_dir: TestSessionDir,
        #[case] engine: Engine,
        #[case] expected_archive_calls: Vec<(String, bool)>,
    ) {
        let session = make_session("session-to-delete", engine);
        super::super::save_session_to(&test_session_dir.sessions_dir, &session)
            .expect("session save should succeed");
        let is_stored = || {
            super::super::load_session_from(&test_session_dir.sessions_dir, "session-to-delete")
                .expect("session lookup should succeed")
                .is_some()
        };
        let archive_calls = RefCell::new(Vec::new());
        let storage_root = test_session_dir.temp_dir.path().join("attachments");

        delete_session_with_archive_in(
            &test_session_dir.sessions_dir,
            &session.session_id,
            Some(&storage_root),
            |session_id| {
                archive_calls
                    .borrow_mut()
                    .push((session_id.to_string(), is_stored()));
                Ok(())
            },
        )
        .expect("session deletion should succeed");

        assert_eq!(
            (archive_calls.into_inner(), is_stored()),
            (expected_archive_calls, false),
        );
    }

    #[rstest]
    fn continues_deleting_when_archive_fails(test_session_dir: TestSessionDir) {
        let session = make_session("session-to-delete", Engine::Codex);
        super::super::save_session_to(&test_session_dir.sessions_dir, &session)
            .expect("session save should succeed");
        let is_stored = || {
            super::super::load_session_from(&test_session_dir.sessions_dir, "session-to-delete")
                .expect("session lookup should succeed")
                .is_some()
        };
        let archive_calls = RefCell::new(Vec::new());
        let storage_root = test_session_dir.temp_dir.path().join("attachments");

        let result = delete_session_with_archive_in(
            &test_session_dir.sessions_dir,
            &session.session_id,
            Some(&storage_root),
            |session_id| {
                archive_calls
                    .borrow_mut()
                    .push((session_id.to_string(), is_stored()));
                Err(anyhow::anyhow!("fixture archive failure"))
            },
        );

        assert_eq!(
            (result.is_ok(), archive_calls.into_inner(), is_stored(),),
            (true, vec![("session-to-delete".to_string(), true)], false,),
        );
    }

    #[rstest]
    fn cleanup_skips_archive_until_incomplete_todos_are_resolved(
        incomplete_todo_session: IncompleteTodoSession,
    ) {
        use super::super::database::{TodoResolution, resolve_agent_todo};

        let IncompleteTodoSession {
            test_session_dir,
            now,
            session_id,
            todo_seq,
        } = incomplete_todo_session;
        let session_id = session_id.as_str();
        let archive_calls = RefCell::new(Vec::new());
        let crit_cleanup_calls = RefCell::new(Vec::new());
        let mut archive_codex_thread = |id: &str| {
            archive_calls.borrow_mut().push(id.to_string());
            Ok(())
        };
        let mut stop_expired_crit_daemons = |session: &Session, _dir: &Path| {
            crit_cleanup_calls
                .borrow_mut()
                .push(session.session_id.clone());
            Ok(())
        };
        let mut cleanup = || {
            cleanup_stale_sessions_in_with_archive_at_storage_and_crit_cleanup(
                &test_session_dir.sessions_dir,
                None,
                |_| None,
                |_| true,
                now,
                &mut archive_codex_thread,
                &mut stop_expired_crit_daemons,
            )
            .expect("session cleanup should succeed")
        };

        let kept_results = vec![cleanup(), cleanup()];
        resolve_agent_todo(
            &test_session_dir.sessions_dir,
            session_id,
            todo_seq,
            TodoResolution::Completed,
            Some("Done"),
        )
        .expect("todo should be resolved");
        let removed = cleanup();
        let is_stored = super::super::load_session_from(&test_session_dir.sessions_dir, session_id)
            .expect("session lookup should succeed")
            .is_some();

        assert_eq!(
            (
                kept_results,
                removed,
                archive_calls.into_inner(),
                crit_cleanup_calls.into_inner(),
                is_stored,
            ),
            (
                vec![false, false],
                true,
                vec![session_id.to_string()],
                vec![session_id.to_string()],
                false,
            ),
        );
    }

    #[rstest]
    fn explicit_delete_skips_archive_while_incomplete_todos_remain(
        incomplete_todo_session: IncompleteTodoSession,
    ) {
        use super::super::database::{TodoResolution, resolve_agent_todo};

        let IncompleteTodoSession {
            test_session_dir,
            session_id,
            todo_seq,
            ..
        } = incomplete_todo_session;
        let session_id = session_id.as_str();
        let archive_calls = RefCell::new(Vec::new());
        let mut archive_codex_thread = |id: &str| {
            archive_calls.borrow_mut().push((
                id.to_string(),
                super::super::load_session_from(&test_session_dir.sessions_dir, id)
                    .expect("session lookup should succeed")
                    .is_some(),
            ));
            Ok(())
        };

        let kept = delete_session_with_archive_in(
            &test_session_dir.sessions_dir,
            session_id,
            None,
            &mut archive_codex_thread,
        )
        .expect("delete should keep a session with incomplete todos");
        resolve_agent_todo(
            &test_session_dir.sessions_dir,
            session_id,
            todo_seq,
            TodoResolution::Completed,
            Some("Done"),
        )
        .expect("todo should be resolved");
        let removed = delete_session_with_archive_in(
            &test_session_dir.sessions_dir,
            session_id,
            None,
            &mut archive_codex_thread,
        )
        .expect("delete should succeed after todos are resolved");
        let absent = delete_session_with_archive_in(
            &test_session_dir.sessions_dir,
            session_id,
            None,
            &mut archive_codex_thread,
        )
        .expect("deleting an absent session should be harmless");

        assert_eq!(
            (kept, removed, absent, archive_calls.into_inner(),),
            (false, true, false, vec![(session_id.to_string(), true)],),
        );
    }

    #[rstest]
    fn explicit_delete_removes_undecodable_session_without_archiving(
        test_session_dir: TestSessionDir,
    ) {
        let session_id = "session-with-invalid-data";
        let session = make_session(session_id, Engine::Codex);
        super::super::save_session_to(&test_session_dir.sessions_dir, &session)
            .expect("session save should succeed");
        super::super::database::open_connection(&test_session_dir.sessions_dir)
            .expect("database should open")
            .execute(
                "UPDATE sessions SET data = ?1 WHERE session_id = ?2",
                ["{", session_id],
            )
            .expect("session data should become undecodable");
        let archive_calls = RefCell::new(Vec::new());

        let removed = delete_session_with_archive_in(
            &test_session_dir.sessions_dir,
            session_id,
            None,
            |id| {
                archive_calls.borrow_mut().push(id.to_string());
                Ok(())
            },
        )
        .expect("undecodable session should still be deletable");
        let row_exists = super::super::database::open_connection(&test_session_dir.sessions_dir)
            .expect("database should open")
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
                [session_id],
                |row| row.get::<_, bool>(0),
            )
            .expect("session row should be queried");

        assert_eq!(
            (removed, archive_calls.into_inner(), row_exists),
            (true, Vec::<String>::new(), false),
        );
    }

    #[rstest]
    fn explicit_delete_does_not_create_a_missing_sessions_directory(
        test_session_dir: TestSessionDir,
    ) {
        let sessions_dir = test_session_dir.temp_dir.path().join("missing-sessions");
        let archive_calls = RefCell::new(Vec::new());

        let removed =
            delete_session_with_archive_in(&sessions_dir, "session-not-found", None, |id| {
                archive_calls.borrow_mut().push(id.to_string());
                Ok(())
            })
            .expect("deleting from a missing directory should be harmless");

        assert_eq!(
            (removed, sessions_dir.exists(), archive_calls.into_inner()),
            (false, false, Vec::<String>::new()),
        );
    }

    #[rstest]
    fn deletes_session_attachments_when_archived_session_is_removed(
        test_session_dir: TestSessionDir,
    ) {
        let session_id = "session-with-files";
        let session = make_session(session_id, Engine::Claude);
        super::super::save_session_to(&test_session_dir.sessions_dir, &session)
            .expect("session save should succeed");
        let storage_root = test_session_dir.temp_dir.path().join("attachments");
        let digest = crate::shared::hex::encode(&sha2::Sha256::digest(session_id.as_bytes()));
        let session_storage = storage_root.join(digest);
        fs::create_dir_all(&session_storage).expect("attachment directory should be created");
        fs::write(session_storage.join("stored-file"), b"fixture bytes")
            .expect("attachment should be written");

        let removed = delete_session_with_archive_in(
            &test_session_dir.sessions_dir,
            session_id,
            Some(&storage_root),
            |_| Ok(()),
        )
        .expect("session deletion should succeed");

        assert_eq!((removed, session_storage.exists()), (true, false));
    }
}
