use std::fs;
use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, TimeDelta, Utc};

use crate::commands::agent::types::{Engine, Session, SessionStatus, TmuxInfo};

#[derive(Clone, Copy)]
pub(crate) enum ArchiveLogContext {
    Store,
    WorktreeCleanup,
}

pub(crate) fn delete_session_with_archive(session_id: &str) -> Result<()> {
    delete_session_with_archive_in(
        &super::sessions_dir()?,
        session_id,
        crate::commands::agent::codex_steer::archive_thread,
    )
}

fn delete_session_with_archive_in(
    sessions_dir: &Path,
    session_id: &str,
    mut archive_codex_thread: impl FnMut(&str) -> anyhow::Result<()>,
) -> Result<()> {
    if let Some(session) = super::load_session_from(sessions_dir, session_id)? {
        archive_codex_thread_before_delete_with(
            &session,
            &mut archive_codex_thread,
            ArchiveLogContext::Store,
        );
    }

    super::delete_session_from(sessions_dir, session_id)
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

pub(super) fn cleanup_stale_sessions_in_with_archive<F, G, A>(
    dir: &Path,
    is_pane_alive: F,
    cwd_exists: G,
    archive_codex_thread: A,
) -> Result<bool>
where
    F: Fn(&TmuxInfo) -> Option<bool>,
    G: Fn(&Path) -> bool,
    A: FnMut(&str) -> anyhow::Result<()>,
{
    cleanup_stale_sessions_in_with_archive_at(
        dir,
        is_pane_alive,
        cwd_exists,
        Utc::now(),
        archive_codex_thread,
    )
}

pub(super) fn cleanup_stale_sessions_in_with_archive_at<F, G, A>(
    dir: &Path,
    is_pane_alive: F,
    cwd_exists: G,
    now: DateTime<Utc>,
    mut archive_codex_thread: A,
) -> Result<bool>
where
    F: Fn(&TmuxInfo) -> Option<bool>,
    G: Fn(&Path) -> bool,
    A: FnMut(&str) -> anyhow::Result<()>,
{
    if !dir.exists() {
        return Ok(false);
    }

    let retention = TimeDelta::days(super::ENDED_SESSION_RETENTION_DAYS);
    let mut changed_any = false;

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }

        let Some(session_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Some(session) = super::load_session_from(dir, session_id)? else {
            continue;
        };

        if session.archived_at.is_some() {
            let session_lock = super::lock_session_for_update(dir, session_id)?;
            if let Some(current) = session_lock.load()?
                && is_expired_archive(&current, now, retention)
            {
                changed_any |=
                    delete_session_and_archive_codex(&current, &path, &mut archive_codex_thread);
            }
            continue;
        }

        if is_expired_ended(&session, now, retention) {
            let session_lock = super::lock_session_for_update(dir, session_id)?;
            if let Some(current) = session_lock.load()?
                && is_expired_ended(&current, now, retention)
            {
                changed_any |=
                    delete_session_and_archive_codex(&current, &path, &mut archive_codex_thread);
            }
            continue;
        }

        let stale_pane = session.status != SessionStatus::Ended
            && session
                .tmux_info
                .as_ref()
                .is_some_and(|info| is_pane_alive(info) == Some(false));
        let orphaned = session.status != SessionStatus::Ended && !cwd_exists(&session.cwd);

        if stale_pane || orphaned {
            let session_lock = super::lock_session_for_update(dir, session_id)?;
            let Some(mut current) = session_lock.load()? else {
                continue;
            };
            if current.archived_at.is_some()
                || current.updated_at != session.updated_at
                || (stale_pane
                    && (current.status != session.status || current.tmux_info != session.tmux_info))
            {
                continue;
            }

            let still_orphaned =
                current.status != SessionStatus::Ended && !cwd_exists(&current.cwd);
            if !still_orphaned && !stale_pane {
                continue;
            }

            current.archived_at = Some(now);
            session_lock.save(&current)?;
            changed_any = true;
        }
    }

    Ok(changed_any)
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

fn delete_session_and_archive_codex(
    session: &Session,
    path: &Path,
    archive_codex_thread: &mut impl FnMut(&str) -> anyhow::Result<()>,
) -> bool {
    archive_codex_thread_before_delete_with(
        session,
        archive_codex_thread,
        ArchiveLogContext::Store,
    );
    let removed = fs::remove_file(path).is_ok();
    let lock_path = path.with_extension("json.lock");
    let _ = fs::remove_file(lock_path);
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{SessionStatus, TmuxInfo};
    use chrono::Utc;
    use rstest::{fixture, rstest};
    use std::cell::RefCell;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    struct TestSessionDir {
        #[expect(dead_code, reason = "kept alive to prevent cleanup until dropped")]
        temp_dir: TempDir,
        sessions_dir: PathBuf,
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

    fn make_session(session_id: &str, engine: Engine) -> Session {
        Session {
            archived_at: None,
            pr_context: None,
            session_id: session_id.to_string(),
            crit_urls: Vec::new(),
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
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            is_delegated: false,
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
        }
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
        let session_path = test_session_dir.sessions_dir.join("session-to-delete.json");
        let archive_calls = RefCell::new(Vec::new());

        delete_session_with_archive_in(
            &test_session_dir.sessions_dir,
            &session.session_id,
            |session_id| {
                archive_calls
                    .borrow_mut()
                    .push((session_id.to_string(), session_path.exists()));
                Ok(())
            },
        )
        .expect("session deletion should succeed");

        assert_eq!(
            (archive_calls.into_inner(), session_path.exists()),
            (expected_archive_calls, false),
        );
    }

    #[rstest]
    fn continues_deleting_when_archive_fails(test_session_dir: TestSessionDir) {
        let session = make_session("session-to-delete", Engine::Codex);
        super::super::save_session_to(&test_session_dir.sessions_dir, &session)
            .expect("session save should succeed");
        let session_path = test_session_dir.sessions_dir.join("session-to-delete.json");
        let archive_calls = RefCell::new(Vec::new());

        let result = delete_session_with_archive_in(
            &test_session_dir.sessions_dir,
            &session.session_id,
            |session_id| {
                archive_calls
                    .borrow_mut()
                    .push((session_id.to_string(), session_path.exists()));
                Err(anyhow::anyhow!("fixture archive failure"))
            },
        );

        assert_eq!(
            (
                result.is_ok(),
                archive_calls.into_inner(),
                session_path.exists(),
            ),
            (true, vec![("session-to-delete".to_string(), true)], false,),
        );
    }
}
