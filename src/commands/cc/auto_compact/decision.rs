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

/// `(cursor_x, cursor_y)` snapshot of a tmux pane's terminal cursor.
pub type CursorPosition = (u32, u32);

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
    /// The pane's terminal cursor moved between Stop hook arm time and
    /// idle-timeout wake time. Either the user typed a follow-up prompt or
    /// the foreground program is still writing output; in both cases firing
    /// `/compact` would interrupt in-flight work.
    UserTyping,
    /// The session's branch already has a merged PR. Compacting work the user
    /// has shipped is wasteful, and the next session on this branch is likely
    /// to be a fresh start anyway.
    BranchMerged,
    /// Less than `idle_timeout` has elapsed since the Stop hook. Happens when
    /// a later Stop hook re-armed the timer but the schedule process from an
    /// earlier turn raced through to wake-up first.
    NotYetElapsed,
    /// Context size on the most recent assistant turn is below
    /// `min_context_tokens`, so compacting now would waste a turn on a
    /// session that has plenty of room left.
    ///
    /// Also returned when the transcript could not be read or did not contain
    /// a usable usage record. Skipping when the size is unknown is the
    /// conservative choice for a feature whose entire point is "don't compact
    /// unless it's worth it".
    BelowThreshold,
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
    /// Terminal cursor position captured when the Stop hook armed this
    /// worker, paired with the position observed at idle-timeout wake.
    /// `None` on either side means we couldn't read it (no tmux info, pane
    /// gone, parse failed); the decision treats unknown as "no movement
    /// detected" so a transient tmux failure doesn't permanently disable
    /// auto-compact.
    pub arm_cursor: Option<CursorPosition>,
    pub wake_cursor: Option<CursorPosition>,
    /// Whether the branch backing this session has a merged PR. `None` when
    /// we couldn't determine it (no git repo, GitHub call failed); treated as
    /// "not merged" so we err on the side of compacting.
    pub branch_merged: Option<bool>,
    /// Context size (in tokens) on the most recent assistant turn:
    /// `input + cache_read + cache_creation + output`. `None` when the
    /// transcript was unreadable or held no usage record — treated as
    /// "below threshold" so we don't compact a session whose state is
    /// unknown.
    pub context_tokens: Option<u64>,
    /// Minimum context size (in tokens) required to fire `/compact`.
    /// Compacting a tiny context discards useful state without freeing any
    /// budget that mattered.
    pub min_context_tokens: u64,
}

pub fn decide_compact(inputs: CompactInputs<'_>) -> CompactDecision {
    if inputs.session.status != SessionStatus::Stopped {
        return CompactDecision::NotStopped;
    }

    if inputs.branch_merged == Some(true) {
        return CompactDecision::BranchMerged;
    }

    // Cursor moved between arm and wake → something happened in the pane
    // (user typed a follow-up, or the foreground program is still writing).
    // Either way, leave the session alone. Missing readings on either side
    // are treated as "no movement" so a one-off tmux failure doesn't block
    // every future compaction.
    if let (Some(arm), Some(wake)) = (inputs.arm_cursor, inputs.wake_cursor)
        && arm != wake
    {
        return CompactDecision::UserTyping;
    }

    let elapsed = inputs.now.signed_duration_since(inputs.session.updated_at);
    let Ok(elapsed_std) = elapsed.to_std() else {
        return CompactDecision::NotYetElapsed;
    };

    if elapsed_std < inputs.idle_timeout {
        return CompactDecision::NotYetElapsed;
    }

    // Threshold check is intentionally last: it relies on transcript I/O the
    // caller may have skipped, and ordering it after the cheap checks keeps
    // the unrelated abort reasons (NotStopped, BranchMerged, ...) reportable
    // even when usage cannot be read.
    match inputs.context_tokens {
        Some(tokens) if tokens >= inputs.min_context_tokens => CompactDecision::Compact,
        _ => CompactDecision::BelowThreshold,
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
            last_bg_task_pending: false,
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
            arm_cursor: None,
            wake_cursor: None,
            branch_merged: None,
            // Default the threshold check out of the way for the existing
            // suite: each older test wants the corresponding cheap check to
            // be the only thing under examination, so feed enough tokens to
            // satisfy a low bar.
            context_tokens: Some(1_000_000),
            min_context_tokens: 1,
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
    #[case::moved_x(Some((10, 5)), Some((11, 5)))]
    #[case::moved_y(Some((10, 5)), Some((10, 6)))]
    #[case::moved_both(Some((10, 5)), Some((0, 0)))]
    fn cursor_movement_aborts(
        #[case] arm: Option<CursorPosition>,
        #[case] wake: Option<CursorPosition>,
    ) {
        // Stop fired an hour ago and idle_timeout has long since elapsed,
        // but the cursor has moved — the user typed a follow-up, or the
        // foreground program is still writing. Compacting now would
        // interrupt whatever is happening.
        let now = Utc::now();
        let session = stopped_session(now - TimeDelta::hours(1));
        let decision = decide_compact(CompactInputs {
            arm_cursor: arm,
            wake_cursor: wake,
            ..inputs(&session, now, Duration::from_secs(60))
        });
        assert_eq!(decision, CompactDecision::UserTyping);
    }

    #[rstest]
    #[case::same_position(Some((10, 5)), Some((10, 5)))]
    #[case::origin(Some((0, 0)), Some((0, 0)))]
    fn stable_cursor_does_not_block(
        #[case] arm: Option<CursorPosition>,
        #[case] wake: Option<CursorPosition>,
    ) {
        let now = Utc::now();
        let session = stopped_session(now - TimeDelta::hours(1));
        let decision = decide_compact(CompactInputs {
            arm_cursor: arm,
            wake_cursor: wake,
            ..inputs(&session, now, Duration::from_secs(60))
        });
        assert_eq!(decision, CompactDecision::Compact);
    }

    #[rstest]
    #[case::arm_missing(None, Some((10, 5)))]
    #[case::wake_missing(Some((10, 5)), None)]
    #[case::both_missing(None, None)]
    fn missing_cursor_does_not_block(
        #[case] arm: Option<CursorPosition>,
        #[case] wake: Option<CursorPosition>,
    ) {
        // A transient tmux failure (pane gone, parse error) on either side
        // must not be treated as "definitely typing", or a single hiccup
        // would permanently disable auto-compact for that pane.
        let now = Utc::now();
        let session = stopped_session(now - TimeDelta::hours(1));
        let decision = decide_compact(CompactInputs {
            arm_cursor: arm,
            wake_cursor: wake,
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
    #[case::exactly_at_threshold(180_000, 180_000, CompactDecision::Compact)]
    #[case::above_threshold(250_000, 180_000, CompactDecision::Compact)]
    #[case::below_threshold(120_000, 180_000, CompactDecision::BelowThreshold)]
    #[case::just_below_threshold(179_999, 180_000, CompactDecision::BelowThreshold)]
    fn context_tokens_drive_threshold(
        #[case] context_tokens: u64,
        #[case] min_context_tokens: u64,
        #[case] expected: CompactDecision,
    ) {
        let now = Utc::now();
        let session = stopped_session(now - TimeDelta::hours(1));
        let decision = decide_compact(CompactInputs {
            context_tokens: Some(context_tokens),
            min_context_tokens,
            ..inputs(&session, now, Duration::from_secs(60))
        });
        assert_eq!(decision, expected);
    }

    #[rstest]
    fn unknown_context_tokens_skips_compact() {
        // Transcript unreadable / no usage record. The whole point of the
        // threshold is "don't compact unless it's worth it", so an unknown
        // size has to fall on the skip side.
        let now = Utc::now();
        let session = stopped_session(now - TimeDelta::hours(1));
        let decision = decide_compact(CompactInputs {
            context_tokens: None,
            min_context_tokens: 180_000,
            ..inputs(&session, now, Duration::from_secs(60))
        });
        assert_eq!(decision, CompactDecision::BelowThreshold);
    }

    #[rstest]
    fn typing_takes_precedence_over_threshold() {
        // Even a fully-loaded context must not preempt the user mid-keystroke.
        let now = Utc::now();
        let session = stopped_session(now - TimeDelta::hours(1));
        let decision = decide_compact(CompactInputs {
            arm_cursor: Some((10, 5)),
            wake_cursor: Some((11, 5)),
            context_tokens: Some(900_000),
            min_context_tokens: 180_000,
            ..inputs(&session, now, Duration::from_secs(60))
        });
        assert_eq!(decision, CompactDecision::UserTyping);
    }

    #[rstest]
    fn merged_takes_precedence_over_typing() {
        // The branch is merged, so we cancel regardless of pane activity.
        // BranchMerged is a stronger signal than UserTyping: typing into a
        // session whose work has already shipped is most likely a stale pane
        // the user forgot about, not in-flight work.
        let now = Utc::now();
        let session = stopped_session(now - TimeDelta::hours(1));
        let decision = decide_compact(CompactInputs {
            arm_cursor: Some((10, 5)),
            wake_cursor: Some((11, 5)),
            branch_merged: Some(true),
            ..inputs(&session, now, Duration::from_secs(60))
        });
        assert_eq!(decision, CompactDecision::BranchMerged);
    }
}
