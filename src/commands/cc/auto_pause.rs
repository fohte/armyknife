//! Logic for automatically pausing long-stopped Claude Code sessions.
//!
//! The actual side effects (signal delivery, store update) live in `sweep.rs`;
//! this module only contains pure functions so they can be unit-tested without
//! spawning processes or touching the filesystem.

use std::time::Duration;

use chrono::{DateTime, Utc};
use thiserror::Error;

use super::types::{Session, SessionStatus};

/// Purely time/status-based pause decision.
///
/// This ignores whether sweep can actually locate the `claude` process -- the
/// caller handles that separately after `Pause` is returned, because pid
/// lookup is a side-effectful tree walk that has no place in a pure function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseDecision {
    /// Session should be paused now (caller must still resolve the pid).
    Pause,
    /// Session is not in a pausable state (e.g., Running, Ended).
    NotStopped,
    /// Session is Stopped but the timeout has not elapsed yet.
    NotYetElapsed,
}

/// Determines whether a stopped session should be paused right now.
///
/// Called by `sweep` once per session on each periodic run. Only `Stopped`
/// sessions whose timeout has elapsed are candidates for pausing; any other
/// state is treated as "user is still using it".
#[cfg(test)]
pub fn decide_pause(session: &Session, now: DateTime<Utc>, timeout: Duration) -> PauseDecision {
    decide_pause_with_effective(session, now, timeout, session.updated_at)
}

/// Like [`decide_pause`] but uses an externally-supplied "last touched"
/// timestamp instead of `session.updated_at`. Sweep passes in
/// `effective_updated_at` (the max of session.updated_at and the tmux pane's
/// last input) so it can account for recent user activity without mutating
/// the session on disk.
pub fn decide_pause_with_effective(
    session: &Session,
    now: DateTime<Utc>,
    timeout: Duration,
    effective_updated_at: DateTime<Utc>,
) -> PauseDecision {
    if session.status != SessionStatus::Stopped {
        return PauseDecision::NotStopped;
    }

    let elapsed = now.signed_duration_since(effective_updated_at);
    let Ok(elapsed_std) = elapsed.to_std() else {
        // Negative elapsed time (clock skew) -- treat as not yet elapsed.
        return PauseDecision::NotYetElapsed;
    };

    if elapsed_std >= timeout {
        PauseDecision::Pause
    } else {
        PauseDecision::NotYetElapsed
    }
}

/// Errors returned by [`parse_duration`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DurationParseError {
    #[error("empty duration string")]
    Empty,
    #[error("invalid duration `{0}`: expected format like `30s`, `10m`, `1h30m`")]
    Invalid(String),
    #[error("unknown duration unit `{0}` (expected s, m, h, or d)")]
    UnknownUnit(String),
    #[error("duration number overflow in `{0}`")]
    Overflow(String),
}

/// Parses a human-friendly duration string like `30s`, `10m`, `1h30m`, `2h`.
///
/// Accepts a sequence of `<number><unit>` pairs where unit is one of:
/// - `s` / `sec` / `secs` / `second` / `seconds`
/// - `m` / `min` / `mins` / `minute` / `minutes`
/// - `h` / `hr` / `hrs` / `hour` / `hours`
/// - `d` / `day` / `days`
///
/// Whitespace between pairs is allowed. A bare number is rejected because the
/// unit is the whole point of this format (ambiguity between seconds and
/// milliseconds has bitten us before).
pub fn parse_duration(input: &str) -> Result<Duration, DurationParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(DurationParseError::Empty);
    }

    let mut total_secs: u64 = 0;
    let mut rest = trimmed;

    while !rest.is_empty() {
        // Skip whitespace between pairs (e.g., "1h 30m").
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }

        // Parse the leading number.
        let digit_end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        if digit_end == 0 {
            return Err(DurationParseError::Invalid(input.to_string()));
        }
        let (num_str, after_num) = rest.split_at(digit_end);
        let value: u64 = num_str
            .parse()
            .map_err(|_| DurationParseError::Overflow(input.to_string()))?;

        // Parse the trailing unit (letters only).
        let after_num = after_num.trim_start();
        let unit_end = after_num
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(after_num.len());
        if unit_end == 0 {
            return Err(DurationParseError::Invalid(input.to_string()));
        }
        let (unit_str, remaining) = after_num.split_at(unit_end);

        let unit_secs: u64 = match unit_str {
            "s" | "sec" | "secs" | "second" | "seconds" => 1,
            "m" | "min" | "mins" | "minute" | "minutes" => 60,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3_600,
            "d" | "day" | "days" => 86_400,
            other => return Err(DurationParseError::UnknownUnit(other.to_string())),
        };

        total_secs = total_secs
            .checked_add(
                value
                    .checked_mul(unit_secs)
                    .ok_or_else(|| DurationParseError::Overflow(input.to_string()))?,
            )
            .ok_or_else(|| DurationParseError::Overflow(input.to_string()))?;

        rest = remaining;
    }

    Ok(Duration::from_secs(total_secs))
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

    #[rstest]
    #[case::exactly_at_timeout(1800, 1800, PauseDecision::Pause)]
    #[case::past_timeout(3600, 1800, PauseDecision::Pause)]
    #[case::not_yet(10, 1800, PauseDecision::NotYetElapsed)]
    fn decide_pause_respects_timeout(
        #[case] elapsed_secs: i64,
        #[case] timeout_secs: u64,
        #[case] expected: PauseDecision,
    ) {
        let now = Utc::now();
        let session = stopped_session(now - TimeDelta::seconds(elapsed_secs));
        let decision = decide_pause(&session, now, Duration::from_secs(timeout_secs));
        assert_eq!(decision, expected);
    }

    #[rstest]
    #[case::running(SessionStatus::Running)]
    #[case::waiting(SessionStatus::WaitingInput)]
    #[case::paused(SessionStatus::Paused)]
    #[case::ended(SessionStatus::Ended)]
    fn decide_pause_skips_non_stopped(#[case] status: SessionStatus) {
        let now = Utc::now();
        let mut session = stopped_session(now - TimeDelta::hours(1));
        session.status = status;
        assert_eq!(
            decide_pause(&session, now, Duration::from_secs(60)),
            PauseDecision::NotStopped
        );
    }

    #[rstest]
    fn decide_pause_clock_skew_is_not_yet_elapsed() {
        let now = Utc::now();
        // updated_at is in the future -- treat as not yet elapsed.
        let session = stopped_session(now + TimeDelta::seconds(60));
        assert_eq!(
            decide_pause(&session, now, Duration::from_secs(10)),
            PauseDecision::NotYetElapsed
        );
    }

    #[rstest]
    #[case::seconds_short("30s", 30)]
    #[case::seconds_long("45seconds", 45)]
    #[case::minutes_short("30m", 1_800)]
    #[case::minutes_long("10min", 600)]
    #[case::hours_short("2h", 7_200)]
    #[case::hours_long("1hour", 3_600)]
    #[case::days("1d", 86_400)]
    #[case::combined("1h30m", 5_400)]
    #[case::combined_with_space("1h 30m", 5_400)]
    #[case::combined_triple("1h2m3s", 3_723)]
    fn parse_duration_accepts_valid(#[case] input: &str, #[case] expected_secs: u64) {
        assert_eq!(
            parse_duration(input).expect("valid duration"),
            Duration::from_secs(expected_secs)
        );
    }

    #[rstest]
    #[case::empty("")]
    #[case::whitespace("   ")]
    #[case::no_unit("30")]
    #[case::unknown_unit("30x")]
    #[case::leading_unit("h30")]
    fn parse_duration_rejects_invalid(#[case] input: &str) {
        assert!(parse_duration(input).is_err());
    }
}
