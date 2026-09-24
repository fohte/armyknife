//! Shared "is this agent session still active?" predicate.
//!
//! `agent sweep` uses this to decide whether to pause a Stopped session;
//! `wm clean` uses it to protect worktrees that still host live sessions
//! from being deleted. Keeping the definition in one place ensures the two
//! features cannot drift apart.
//!
//! A session is **active** unless `auto_pause::decide_pause_with_effective`
//! says it should be paused right now -- i.e. anything that is NOT a
//! clean `PauseDecision::Pause` (composer draft, pending bg task, non-Stopped
//! status) counts as active.

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::commands::agent::auto_pause::{PauseDecision, decide_pause_with_effective};
use crate::commands::agent::pane;
#[cfg(test)]
use crate::commands::agent::types::Engine;
use crate::commands::agent::types::{Session, SessionStatus};

/// Source of the timestamp that should keep a stopped session from pausing.
///
/// Pulled out as a trait so tests can inject deterministic timestamps
/// instead of poking real tmux panes.
pub trait ActivityProbe {
    fn last_activity_at(&self, session: &Session, now: DateTime<Utc>) -> Option<DateTime<Utc>>;
}

/// Probe that always reports "no observation". Useful when tmux is not
/// available or when the caller does not want activity to influence the
/// decision (e.g., a non-interactive sweep over disk only).
pub struct NoActivityProbe;

impl ActivityProbe for NoActivityProbe {
    fn last_activity_at(&self, _: &Session, _: DateTime<Utc>) -> Option<DateTime<Utc>> {
        None
    }
}

/// Production probe that checks whether the live composer contains a draft.
pub struct TmuxActivityProbe;

impl ActivityProbe for TmuxActivityProbe {
    fn last_activity_at(&self, session: &Session, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let pane_id = &session.tmux_info.as_ref()?.pane_id;
        pane::input::pane_has_draft(pane_id, session.engine)
            .filter(|has_draft| *has_draft)
            .map(|_| now)
    }
}

/// Returns the later of `session.updated_at` and the time a composer draft
/// was observed. Falls back to `session.updated_at` when no draft is known.
pub fn effective_updated_at<P: ActivityProbe>(
    session: &Session,
    probe: &P,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    match probe.last_activity_at(session, now) {
        Some(activity_at) if activity_at > session.updated_at => activity_at,
        _ => session.updated_at,
    }
}

/// Whether a session is still "active" by the auto-pause definition.
///
/// Active means: NOT in a state where sweep would pause it. Ended sessions
/// are never active. Paused sessions stay paused -- they don't currently
/// host a live agent process, so treat them as inactive for the purposes
/// of worktree protection.
pub fn is_session_active<P: ActivityProbe>(
    session: &Session,
    probe: &P,
    now: DateTime<Utc>,
    timeout: Duration,
) -> bool {
    if matches!(session.status, SessionStatus::Ended | SessionStatus::Paused) {
        return false;
    }
    let effective = effective_updated_at(session, probe, now);
    !matches!(
        decide_pause_with_effective(session, now, timeout, effective),
        PauseDecision::Pause
    )
}

/// Whether any session in `sessions` is active and lives inside
/// `worktree_path` (i.e. its cwd is the worktree or a descendant).
pub fn contains_active_session<P: ActivityProbe>(
    worktree_path: &Path,
    sessions: &[Session],
    probe: &P,
    now: DateTime<Utc>,
    timeout: Duration,
) -> bool {
    // Canonicalize both sides: on macOS `/tmp` and `/var` are symlinks to
    // `/private/tmp` and `/private/var`, so a session whose cwd was captured
    // pre-resolution would not match a worktree path read from git which
    // resolves them. Fall back to the raw path on failure so a missing
    // directory just behaves like the previous prefix check.
    let canonical_worktree = worktree_path
        .canonicalize()
        .unwrap_or_else(|_| worktree_path.to_path_buf());
    sessions
        .iter()
        .filter(|s| {
            let canonical_cwd = s.cwd.canonicalize().unwrap_or_else(|_| s.cwd.clone());
            canonical_cwd.starts_with(&canonical_worktree)
        })
        .any(|s| is_session_active(s, probe, now, timeout))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::{BTreeSet, HashMap};
    use std::path::PathBuf;

    use chrono::TimeDelta;
    use rstest::rstest;

    use super::*;

    #[derive(Default)]
    pub(crate) struct FakeActivityProbe {
        activity: RefCell<HashMap<String, DateTime<Utc>>>,
    }

    impl FakeActivityProbe {
        pub(crate) fn with(pairs: &[(&str, DateTime<Utc>)]) -> Self {
            let p = Self::default();
            for (id, ts) in pairs {
                p.activity.borrow_mut().insert((*id).to_string(), *ts);
            }
            p
        }
    }

    impl ActivityProbe for FakeActivityProbe {
        fn last_activity_at(
            &self,
            session: &Session,
            _now: DateTime<Utc>,
        ) -> Option<DateTime<Utc>> {
            self.activity.borrow().get(&session.session_id).copied()
        }
    }

    fn make_session(id: &str, status: SessionStatus, updated_at: DateTime<Utc>) -> Session {
        make_session_at(id, status, updated_at, PathBuf::from("/tmp/wt"))
    }

    fn make_session_at(
        id: &str,
        status: SessionStatus,
        updated_at: DateTime<Utc>,
        cwd: PathBuf,
    ) -> Session {
        Session {
            session_id: id.to_string(),
            cwd,
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: updated_at,
            updated_at,
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            engine: Engine::Claude,
        }
    }

    #[rstest]
    #[case::running(SessionStatus::Running, true)]
    #[case::waiting(SessionStatus::WaitingInput, true)]
    #[case::stopped_recent(SessionStatus::Stopped, true)]
    fn active_when_status_or_recent(#[case] status: SessionStatus, #[case] expected: bool) {
        let now = Utc::now();
        let session = make_session("s", status, now);
        let probe = NoActivityProbe;
        assert_eq!(
            is_session_active(&session, &probe, now, Duration::from_secs(60)),
            expected
        );
    }

    #[rstest]
    #[case::paused(SessionStatus::Paused)]
    #[case::ended(SessionStatus::Ended)]
    fn paused_and_ended_are_not_active(#[case] status: SessionStatus) {
        let now = Utc::now();
        let session = make_session("s", status, now - TimeDelta::hours(1));
        let probe = NoActivityProbe;
        assert!(!is_session_active(
            &session,
            &probe,
            now,
            Duration::from_secs(60)
        ));
    }

    #[rstest]
    fn stopped_past_timeout_is_inactive() {
        let now = Utc::now();
        let session = make_session("s", SessionStatus::Stopped, now - TimeDelta::hours(1));
        let probe = NoActivityProbe;
        assert!(!is_session_active(
            &session,
            &probe,
            now,
            Duration::from_secs(60)
        ));
    }

    #[rstest]
    fn stopped_with_pending_bg_task_is_active() {
        let now = Utc::now();
        let mut session = make_session("s", SessionStatus::Stopped, now - TimeDelta::hours(1));
        session.pending_bg_task_ids.insert("bg".to_string());
        let probe = NoActivityProbe;
        assert!(is_session_active(
            &session,
            &probe,
            now,
            Duration::from_secs(60)
        ));
    }

    #[rstest]
    fn recent_draft_observation_keeps_stopped_active() {
        let now = Utc::now();
        let session = make_session("s", SessionStatus::Stopped, now - TimeDelta::hours(1));
        let recent = now - TimeDelta::seconds(5);
        let probe = FakeActivityProbe::with(&[("s", recent)]);
        assert!(is_session_active(
            &session,
            &probe,
            now,
            Duration::from_secs(60)
        ));
    }

    #[rstest]
    fn contains_active_session_matches_by_cwd_prefix() {
        let now = Utc::now();
        let wt = PathBuf::from("/tmp/wt-a");

        // Active session inside the worktree.
        let s_active = make_session_at("a", SessionStatus::Running, now, wt.join("src"));
        // Active session in a different worktree -- must not match.
        let s_other = make_session_at("b", SessionStatus::Running, now, PathBuf::from("/tmp/wt-b"));
        // Inactive session inside the worktree -- must not block deletion.
        let s_ended = make_session_at("c", SessionStatus::Ended, now, wt.clone());

        let probe = NoActivityProbe;
        let timeout = Duration::from_secs(60);

        assert!(contains_active_session(
            &wt,
            &[s_active.clone(), s_other.clone(), s_ended.clone()],
            &probe,
            now,
            timeout
        ));
        assert!(!contains_active_session(
            &wt,
            &[s_other, s_ended],
            &probe,
            now,
            timeout
        ));
    }
}
