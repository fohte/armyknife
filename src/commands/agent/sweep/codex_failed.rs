use std::path::Path;

use anyhow::Result;
use chrono::Utc;

use super::super::codex_steer::Client;
use super::super::store;
use super::super::tmux_sync::TmuxStatusSyncer;
use super::super::types::{Engine, SessionStatus};
use super::session_files::session_ids_in;

#[derive(Default)]
struct AppServerTurnStatusReader {
    client: Option<Client>,
    connection_attempted: bool,
}

impl AppServerTurnStatusReader {
    fn latest_turn_status(&mut self, thread_id: &str) -> Result<Option<String>> {
        if !self.connection_attempted {
            self.connection_attempted = true;
            self.client = Some(Client::connect()?);
        }

        let Some(client) = &mut self.client else {
            return Ok(None);
        };
        let result = client.latest_turn_status(thread_id);
        if result.is_err() {
            self.client = None;
        }
        result
    }
}

pub(super) fn mark_failed_codex_sessions<T: TmuxStatusSyncer>(
    sessions_dir: &Path,
    syncer: &T,
    dry_run: bool,
) -> Result<usize> {
    let mut reader = AppServerTurnStatusReader::default();
    mark_failed_codex_sessions_with_reader(sessions_dir, syncer, dry_run, |thread_id| {
        reader.latest_turn_status(thread_id)
    })
}

fn mark_failed_codex_sessions_with_reader<T, F>(
    sessions_dir: &Path,
    syncer: &T,
    dry_run: bool,
    mut read_latest_turn_status: F,
) -> Result<usize>
where
    T: TmuxStatusSyncer,
    F: FnMut(&str) -> Result<Option<String>>,
{
    let mut failed = 0;

    for session_id in session_ids_in(sessions_dir)? {
        let Some(session) = store::load_session_from(sessions_dir, &session_id)? else {
            continue;
        };
        if session.engine != Engine::Codex || session.status != SessionStatus::Running {
            continue;
        }

        let expected_updated_at = session.updated_at;
        let latest_status = match read_latest_turn_status(&session.session_id) {
            Ok(Some(status)) => status,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    event = "agent.sweep.codex_turn_read_failed",
                    session = %session.session_id,
                    error = %error,
                );
                continue;
            }
        };
        if latest_status != "failed" {
            continue;
        }

        if dry_run {
            tracing::info!(
                event = "agent.sweep.dry_run_codex_turn_failed",
                session = %session.session_id,
            );
            failed += 1;
            continue;
        }

        let mut failed_session = None;
        store::update_session_in(sessions_dir, &session.session_id, |current| {
            if current.engine != Engine::Codex
                || current.status != SessionStatus::Running
                || current.updated_at != expected_updated_at
            {
                return false;
            }

            current.status = SessionStatus::Failed;
            current.updated_at = Utc::now();
            failed_session = Some(current.clone());
            true
        })?;

        if let Some(session) = failed_session {
            tracing::info!(
                event = "agent.sweep.codex_turn_failed",
                session = %session.session_id,
            );
            syncer.sync(
                session.tmux_info.as_ref().map(|info| info.pane_id.as_str()),
                Some(SessionStatus::Failed),
                Some(&session),
                None,
                sessions_dir,
            );
            failed += 1;
        }
    }

    Ok(failed)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use chrono::{DateTime, TimeDelta, Utc};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;
    use crate::commands::agent::store::save_session_to;
    use crate::commands::agent::tmux_sync::test_support::{RecordingTmuxStatusSyncer, SyncCall};
    use crate::commands::agent::types::{Session, TmuxInfo};

    #[fixture]
    fn sessions_dir() -> TempDir {
        TempDir::new().expect("sessions dir")
    }

    fn session(
        session_id: &str,
        status: SessionStatus,
        engine: Engine,
        updated_at: DateTime<Utc>,
    ) -> Session {
        Session {
            pr_context: None,
            session_id: session_id.to_string(),
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp/fixture-workspace"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: updated_at,
            updated_at,
            last_message: Some("previous response".to_string()),
            current_tool: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            work_type_pinned: false,
            is_delegated: false,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine,
        }
    }

    fn load_session(sessions_dir: &Path, session_id: &str) -> Session {
        store::load_session_from(sessions_dir, session_id)
            .expect("load session")
            .expect("session exists")
    }

    #[rstest]
    fn marks_failed_running_codex_turn_and_syncs_tmux(sessions_dir: TempDir) {
        let mut session = session(
            "fixture-thread-id",
            SessionStatus::Running,
            Engine::Codex,
            Utc::now(),
        );
        session.tmux_info = Some(TmuxInfo {
            session_name: "fixture-session".to_string(),
            window_name: "fixture-window".to_string(),
            window_index: 0,
            pane_id: "%73".to_string(),
            socket_path: None,
        });
        save_session_to(sessions_dir.path(), &session).expect("save session");

        let syncer = RecordingTmuxStatusSyncer::default();
        let mut requested_threads = Vec::new();
        let failed = mark_failed_codex_sessions_with_reader(
            sessions_dir.path(),
            &syncer,
            false,
            |thread_id| {
                requested_threads.push(thread_id.to_string());
                Ok(Some("failed".to_string()))
            },
        )
        .expect("mark failed turn");

        let reloaded = load_session(sessions_dir.path(), "fixture-thread-id");
        let actual = (
            failed,
            (reloaded.status, reloaded.last_message),
            requested_threads,
            syncer.calls.borrow().clone(),
        );
        let expected_sync: Vec<SyncCall> = vec![(
            Some("%73".to_string()),
            Some(SessionStatus::Failed),
            Some("fixture-thread-id".to_string()),
            sessions_dir.path().to_path_buf(),
        )];
        assert_eq!(
            actual,
            (
                1,
                (SessionStatus::Failed, Some("previous response".to_string())),
                vec!["fixture-thread-id".to_string()],
                expected_sync,
            ),
        );
    }

    #[rstest]
    #[case::latest_completed(SessionStatus::Running, Engine::Codex, Some("completed"), true)]
    #[case::latest_in_progress(SessionStatus::Running, Engine::Codex, Some("inProgress"), true)]
    #[case::latest_interrupted(SessionStatus::Running, Engine::Codex, Some("interrupted"), true)]
    #[case::no_turn(SessionStatus::Running, Engine::Codex, None, true)]
    #[case::not_running(SessionStatus::Stopped, Engine::Codex, Some("failed"), false)]
    #[case::not_codex(SessionStatus::Running, Engine::Claude, Some("failed"), false)]
    #[case::already_failed(SessionStatus::Failed, Engine::Codex, Some("failed"), false)]
    fn ignores_sessions_without_a_running_failed_codex_turn(
        sessions_dir: TempDir,
        #[case] initial_status: SessionStatus,
        #[case] engine: Engine,
        #[case] latest_status: Option<&str>,
        #[case] should_query: bool,
    ) {
        let session = session("fixture-session-id", initial_status, engine, Utc::now());
        save_session_to(sessions_dir.path(), &session).expect("save session");

        let syncer = RecordingTmuxStatusSyncer::default();
        let mut requested_threads = Vec::new();
        let failed = mark_failed_codex_sessions_with_reader(
            sessions_dir.path(),
            &syncer,
            false,
            |thread_id| {
                requested_threads.push(thread_id.to_string());
                Ok(latest_status.map(str::to_string))
            },
        )
        .expect("inspect turn status");

        assert_eq!(
            (
                failed,
                load_session(sessions_dir.path(), "fixture-session-id").status,
                requested_threads,
                syncer.calls.borrow().clone(),
            ),
            (
                0,
                initial_status,
                should_query
                    .then(|| "fixture-session-id".to_string())
                    .into_iter()
                    .collect(),
                Vec::<SyncCall>::new(),
            ),
        );
    }

    #[rstest]
    fn reader_error_does_not_stop_other_sessions(sessions_dir: TempDir) {
        for session_id in ["fixture-error-session", "fixture-failed-session"] {
            save_session_to(
                sessions_dir.path(),
                &session(
                    session_id,
                    SessionStatus::Running,
                    Engine::Codex,
                    Utc::now(),
                ),
            )
            .expect("save session");
        }

        let syncer = RecordingTmuxStatusSyncer::default();
        let mut requested_threads = Vec::new();
        let failed = mark_failed_codex_sessions_with_reader(
            sessions_dir.path(),
            &syncer,
            false,
            |thread_id| {
                requested_threads.push(thread_id.to_string());
                if thread_id == "fixture-error-session" {
                    anyhow::bail!("fixture app-server unavailable");
                }
                Ok(Some("failed".to_string()))
            },
        )
        .expect("reader failures must not fail sweep");
        requested_threads.sort();

        assert_eq!(
            (
                failed,
                load_session(sessions_dir.path(), "fixture-error-session").status,
                load_session(sessions_dir.path(), "fixture-failed-session").status,
                requested_threads,
            ),
            (
                1,
                SessionStatus::Running,
                SessionStatus::Failed,
                vec![
                    "fixture-error-session".to_string(),
                    "fixture-failed-session".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    fn does_not_overwrite_a_session_updated_during_app_server_read(sessions_dir: TempDir) {
        let original_updated_at = Utc::now();
        save_session_to(
            sessions_dir.path(),
            &session(
                "fixture-thread-id",
                SessionStatus::Running,
                Engine::Codex,
                original_updated_at,
            ),
        )
        .expect("save session");
        let newer_updated_at = original_updated_at + TimeDelta::seconds(1);

        let syncer = RecordingTmuxStatusSyncer::default();
        let failed =
            mark_failed_codex_sessions_with_reader(sessions_dir.path(), &syncer, false, |_| {
                store::update_session_in(sessions_dir.path(), "fixture-thread-id", |current| {
                    current.updated_at = newer_updated_at;
                    current.last_message = Some("new response".to_string());
                    true
                })?;
                Ok(Some("failed".to_string()))
            })
            .expect("inspect turn status");
        let reloaded = load_session(sessions_dir.path(), "fixture-thread-id");

        assert_eq!(
            (
                failed,
                reloaded.status,
                reloaded.updated_at,
                reloaded.last_message,
                syncer.calls.borrow().clone(),
            ),
            (
                0,
                SessionStatus::Running,
                newer_updated_at,
                Some("new response".to_string()),
                Vec::<SyncCall>::new(),
            ),
        );
    }

    #[rstest]
    fn dry_run_reports_failed_turn_without_saving_or_syncing(sessions_dir: TempDir) {
        save_session_to(
            sessions_dir.path(),
            &session(
                "fixture-thread-id",
                SessionStatus::Running,
                Engine::Codex,
                Utc::now(),
            ),
        )
        .expect("save session");
        let syncer = RecordingTmuxStatusSyncer::default();

        let failed =
            mark_failed_codex_sessions_with_reader(sessions_dir.path(), &syncer, true, |_| {
                Ok(Some("failed".to_string()))
            })
            .expect("inspect turn status");

        assert_eq!(
            (
                failed,
                load_session(sessions_dir.path(), "fixture-thread-id").status,
                syncer.calls.borrow().clone(),
            ),
            (1, SessionStatus::Running, Vec::<SyncCall>::new()),
        );
    }
}
