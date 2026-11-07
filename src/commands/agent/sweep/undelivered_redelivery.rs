use std::path::Path;
use std::time::{Duration, SystemTime};

use anyhow::Result;

use super::super::bg;
use super::super::store;
use super::super::types::{Session, SessionStatus};

/// Older messages wait for the session's next resume, so sweep does not wake
/// sessions nobody has needed for days.
const MAX_MESSAGE_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Starts a detached redelivery for each session with a recent undelivered
/// notification and returns how many were started. A session stopped or paused
/// by sweep gets no hook event until something wakes it, so without this pass
/// nothing would ever retry its saved messages.
pub(super) fn spawn_recent(sessions_dir: &Path, dry_run: bool) -> Result<usize> {
    let since = SystemTime::now()
        .checked_sub(MAX_MESSAGE_AGE)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    spawn_recent_with(
        sessions_dir,
        dry_run,
        |session_id| bg::has_undelivered_since(session_id, since),
        bg::spawn_redelivery,
    )
}

fn spawn_recent_with(
    sessions_dir: &Path,
    dry_run: bool,
    mut has_recent: impl FnMut(&str) -> Result<bool>,
    mut spawn: impl FnMut(&str),
) -> Result<usize> {
    let mut started = 0;
    for session in store::list_all_sessions_from(sessions_dir)? {
        if !can_receive_redelivery(&session) {
            continue;
        }
        match has_recent(&session.session_id) {
            Ok(true) => {
                tracing::info!(
                    event = "agent.sweep.redelivery_started",
                    session = %session.session_id,
                    dry_run,
                );
                if !dry_run {
                    spawn(&session.session_id);
                }
                started += 1;
            }
            Ok(false) => {}
            Err(error) => tracing::warn!(
                event = "agent.sweep.redelivery_scan_failed",
                session = %session.session_id,
                error = %format!("{error:#}"),
            ),
        }
    }
    Ok(started)
}

fn can_receive_redelivery(session: &Session) -> bool {
    // A signaled process is shutting down, so its socket could accept a message
    // and lose it. A later sweep confirms it `Paused`, and notify then wakes it.
    session.status != SessionStatus::Ended && !session.sweep_signaled
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use chrono::Utc;
    use rstest::rstest;
    use tempfile::TempDir;

    use super::*;

    fn make_session(id: &str, status: SessionStatus) -> Session {
        let timestamp = Utc::now();
        Session {
            archived_at: None,
            pr_context: None,
            session_id: id.to_string(),
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/worktree"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: timestamp,
            updated_at: timestamp,
            last_message: None,
            current_tool: None,
            label: None,
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: BTreeMap::new(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Default::default(),
            model: None,
            effort: None,
        }
    }

    fn sessions_dir(sessions: &[Session]) -> TempDir {
        let dir = TempDir::new().expect("temporary directory should be created");
        for session in sessions {
            store::save_session_to(dir.path(), session).expect("session should be saved");
        }
        dir
    }

    fn run(
        sessions: &[Session],
        dry_run: bool,
        has_recent: impl FnMut(&str) -> Result<bool>,
    ) -> (usize, Vec<String>) {
        let dir = sessions_dir(sessions);
        let mut spawned = Vec::new();
        let started = spawn_recent_with(dir.path(), dry_run, has_recent, |session_id| {
            spawned.push(session_id.to_string())
        })
        .expect("pass should succeed");
        (started, spawned)
    }

    #[rstest]
    #[case::running(make_session("fixture", SessionStatus::Running), true)]
    #[case::stopped(make_session("fixture", SessionStatus::Stopped), true)]
    #[case::paused(make_session("fixture", SessionStatus::Paused), true)]
    #[case::ended(make_session("fixture", SessionStatus::Ended), false)]
    #[case::archived(
        Session { archived_at: Some(Utc::now()), ..make_session("fixture", SessionStatus::Paused) },
        false
    )]
    #[case::shutting_down(
        Session { sweep_signaled: true, ..make_session("fixture", SessionStatus::Stopped) },
        false
    )]
    fn starts_redelivery_only_for_sessions_that_can_receive_it(
        #[case] session: Session,
        #[case] expected: bool,
    ) {
        let actual = run(&[session], false, |_| Ok(true));

        let expected = if expected {
            (1, vec!["fixture".to_string()])
        } else {
            (0, Vec::new())
        };
        assert_eq!(actual, expected);
    }

    #[test]
    fn skips_sessions_without_recent_messages() {
        let actual = run(
            &[make_session("fixture", SessionStatus::Paused)],
            false,
            |_| Ok(false),
        );

        assert_eq!(actual, (0, Vec::new()));
    }

    #[test]
    fn dry_run_counts_sessions_without_starting_redelivery() {
        let actual = run(
            &[make_session("fixture", SessionStatus::Paused)],
            true,
            |_| Ok(true),
        );

        assert_eq!(actual, (1, Vec::new()));
    }

    #[test]
    fn scan_failure_does_not_block_other_sessions() {
        let actual = run(
            &[
                make_session("fixture-unreadable", SessionStatus::Paused),
                make_session("fixture-readable", SessionStatus::Paused),
            ],
            false,
            |session_id| {
                if session_id == "fixture-unreadable" {
                    anyhow::bail!("fixture scan failure");
                }
                Ok(true)
            },
        );

        assert_eq!(actual, (1, vec!["fixture-readable".to_string()]));
    }
}
