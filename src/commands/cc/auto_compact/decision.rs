//! Pure decision logic for auto-compact.
//!
//! Mirrors `auto_pause::decide_pause_with_effective` in spirit: side effects
//! (sleeping, signaling, exec'ing `claude -r`) live in the schedule subcommand;
//! this module owns the question "given these inputs, should we compact?" so
//! that the policy can be unit-tested without touching the filesystem, tmux,
//! or the network.

use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::commands::cc::types::{Session, SessionStatus};

/// What the schedule subcommand should do at wake-up time.
///
/// `Compact` is the happy path; every other variant explains why we bailed,
/// which is useful both for tests and for the runtime log that a long-running
/// schedule process writes when it decides to abort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactDecision {
    /// All conditions hold; SIGTERM and run `/compact`.
    Compact,
    /// Session has moved out of Stopped (user came back, or sweep already
    /// paused it, or it ended). Cancelling silently is the right move because
    /// either the user resumed work or another worker handled the session.
    NotStopped,
    /// User has typed something into the pane after the Stop hook fired
    /// (pty atime > Stop time). Killing them mid-prompt would be hostile.
    UserTyping,
    /// The session's branch already has a merged PR. Compacting work the user
    /// has shipped is wasteful, and the next session on this branch is likely
    /// to be a fresh start anyway.
    BranchMerged,
    /// Less than `idle_timeout` has elapsed since the Stop hook. Happens when
    /// a later Stop hook re-armed the timer but the schedule process from an
    /// earlier turn raced through to wake-up first.
    NotYetElapsed,
}

/// Inputs to the decision, gathered by the caller before calling.
///
/// Kept as an explicit struct (rather than a long parameter list) because
/// schedule's wake-up path collects each value from a different subsystem
/// (store, tmux, git/github) and the named fields make the call site readable.
#[derive(Debug, Clone, Copy)]
pub struct CompactInputs<'a> {
    pub session: &'a Session,
    pub now: DateTime<Utc>,
    pub idle_timeout: Duration,
    /// PTY atime for the session's tmux pane, if available. `None` means we
    /// could not query it (no tmux info, pane gone, stat failed). Treated as
    /// "no recent input" rather than "definitely typing" — falsely cancelling
    /// would defeat the whole feature.
    pub last_input: Option<DateTime<Utc>>,
    /// Whether the branch backing this session has a merged PR. `None` when
    /// we couldn't determine it (no git repo, GitHub call failed); treated as
    /// "not merged" so we err on the side of compacting.
    pub branch_merged: Option<bool>,
}

pub fn decide_compact(inputs: CompactInputs<'_>) -> CompactDecision {
    if inputs.session.status != SessionStatus::Stopped {
        return CompactDecision::NotStopped;
    }

    if inputs.branch_merged == Some(true) {
        return CompactDecision::BranchMerged;
    }

    if let Some(input_at) = inputs.last_input
        && input_at > inputs.session.updated_at
    {
        return CompactDecision::UserTyping;
    }

    let elapsed = inputs.now.signed_duration_since(inputs.session.updated_at);
    let Ok(elapsed_std) = elapsed.to_std() else {
        return CompactDecision::NotYetElapsed;
    };

    if elapsed_std >= inputs.idle_timeout {
        CompactDecision::Compact
    } else {
        CompactDecision::NotYetElapsed
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::TimeDelta;
    use rstest::rstest;

    use super::*;

    fn stopped_session(updated_at: DateTime<Utc>) -> Session {
        Session {
            session_id: "sess".to_string(),
            cwd: PathBuf::from("/tmp"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
            created_at: updated_at,
            updated_at,
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
        }
    }

    fn inputs<'a>(
        session: &'a Session,
        now: DateTime<Utc>,
        idle_timeout: Duration,
    ) -> CompactInputs<'a> {
        CompactInputs {
            session,
            now,
            idle_timeout,
            last_input: None,
            branch_merged: None,
        }
    }

    #[rstest]
    #[case::exactly_at_timeout(270, 270, CompactDecision::Compact)]
    #[case::past_timeout(600, 270, CompactDecision::Compact)]
    #[case::not_yet(10, 270, CompactDecision::NotYetElapsed)]
    fn elapsed_drives_compact(
        #[case] elapsed_secs: i64,
        #[case] timeout_secs: u64,
        #[case] expected: CompactDecision,
    ) {
        let now = Utc::now();
        let session = stopped_session(now - TimeDelta::seconds(elapsed_secs));
        let decision = decide_compact(inputs(&session, now, Duration::from_secs(timeout_secs)));
        assert_eq!(decision, expected);
    }

    #[rstest]
    #[case::running(SessionStatus::Running)]
    #[case::waiting(SessionStatus::WaitingInput)]
    #[case::paused(SessionStatus::Paused)]
    #[case::ended(SessionStatus::Ended)]
    fn non_stopped_status_aborts(#[case] status: SessionStatus) {
        let now = Utc::now();
        let mut session = stopped_session(now - TimeDelta::hours(1));
        session.status = status;
        let decision = decide_compact(inputs(&session, now, Duration::from_secs(60)));
        assert_eq!(decision, CompactDecision::NotStopped);
    }

    #[rstest]
    fn user_typing_aborts_even_when_elapsed() {
        // Stop fired an hour ago, but the user hit a key 5s ago — they're
        // composing a follow-up prompt. Killing them mid-keystroke is exactly
        // the rude behavior we're trying to avoid.
        let now = Utc::now();
        let session = stopped_session(now - TimeDelta::hours(1));
        let last_input = now - TimeDelta::seconds(5);
        let decision = decide_compact(CompactInputs {
            last_input: Some(last_input),
            ..inputs(&session, now, Duration::from_secs(60))
        });
        assert_eq!(decision, CompactDecision::UserTyping);
    }

    #[rstest]
    fn last_input_at_or_before_stop_does_not_block() {
        // last_input that predates updated_at (or equals it, e.g. when claude
        // wrote its own prompt to the pane during Stop) must not be treated
        // as "user typing".
        let now = Utc::now();
        let stop_time = now - TimeDelta::hours(1);
        let session = stopped_session(stop_time);
        let decision = decide_compact(CompactInputs {
            last_input: Some(stop_time),
            ..inputs(&session, now, Duration::from_secs(60))
        });
        assert_eq!(decision, CompactDecision::Compact);
    }

    #[rstest]
    fn branch_merged_aborts() {
        let now = Utc::now();
        let session = stopped_session(now - TimeDelta::hours(1));
        let decision = decide_compact(CompactInputs {
            branch_merged: Some(true),
            ..inputs(&session, now, Duration::from_secs(60))
        });
        assert_eq!(decision, CompactDecision::BranchMerged);
    }

    #[rstest]
    #[case::unknown_state(None)]
    #[case::explicit_false(Some(false))]
    fn non_merged_branch_does_not_block(#[case] branch_merged: Option<bool>) {
        let now = Utc::now();
        let session = stopped_session(now - TimeDelta::hours(1));
        let decision = decide_compact(CompactInputs {
            branch_merged,
            ..inputs(&session, now, Duration::from_secs(60))
        });
        assert_eq!(decision, CompactDecision::Compact);
    }

    #[rstest]
    fn clock_skew_is_not_yet_elapsed() {
        let now = Utc::now();
        // updated_at in the future (NTP just stepped backwards on this box).
        let session = stopped_session(now + TimeDelta::seconds(60));
        let decision = decide_compact(inputs(&session, now, Duration::from_secs(10)));
        assert_eq!(decision, CompactDecision::NotYetElapsed);
    }

    #[rstest]
    fn merged_takes_precedence_over_typing() {
        // The branch is merged, so we cancel regardless of pty activity.
        // BranchMerged is a stronger signal than UserTyping: typing into a
        // session whose work has already shipped is most likely a stale pane
        // the user forgot about, not in-flight work.
        let now = Utc::now();
        let session = stopped_session(now - TimeDelta::hours(1));
        let decision = decide_compact(CompactInputs {
            last_input: Some(now - TimeDelta::seconds(2)),
            branch_merged: Some(true),
            ..inputs(&session, now, Duration::from_secs(60))
        });
        assert_eq!(decision, CompactDecision::BranchMerged);
    }
}
