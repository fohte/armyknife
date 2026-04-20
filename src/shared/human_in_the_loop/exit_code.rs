//! Exit codes for review commands.

use super::HumanInTheLoopError;

/// The user approved the review (success).
/// Not used directly (successful return implies exit code 0), but defined for completeness.
#[expect(
    dead_code,
    reason = "defined for documentation; success exits via Ok(())"
)]
pub const APPROVED: i32 = 0;

/// The user did not approve the review (closed the editor without approving).
pub const NOT_APPROVED: i32 = 1;

/// The editor is already open for this file (lock exists).
pub const ALREADY_OPEN: i32 = 2;

/// The terminal emulator failed to launch (e.g., macOS asleep, Ghostty
/// initialization error). Callers can retry once the system wakes.
pub const TERMINAL_LAUNCH_FAILED: i32 = 3;

/// Convert a `HumanInTheLoopError` from `start_review` into an `anyhow::Result`
/// that `std::process::exit`s with `TERMINAL_LAUNCH_FAILED` when the error is
/// a terminal launch timeout, and propagates all other errors unchanged.
///
/// This keeps the mapping in one place so each review command handler can
/// share the same behavior.
pub fn exit_on_terminal_launch_failure<T>(result: super::Result<T>) -> anyhow::Result<T>
where
    anyhow::Error: From<HumanInTheLoopError>,
{
    match result {
        Ok(value) => Ok(value),
        Err(e @ HumanInTheLoopError::TerminalLaunchFailed { .. }) => {
            eprintln!("{e}");
            std::process::exit(TERMINAL_LAUNCH_FAILED);
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_on_terminal_launch_failure_passes_through_ok() {
        let value: anyhow::Result<i32> = exit_on_terminal_launch_failure(Ok(42));
        assert_eq!(value.expect("ok"), 42);
    }

    #[test]
    fn exit_on_terminal_launch_failure_propagates_other_errors() {
        let input: super::super::Result<i32> = Err(HumanInTheLoopError::NotApproved);
        let err = exit_on_terminal_launch_failure(input).expect_err("expected propagated error");
        assert!(err.to_string().contains("Not approved"));
    }
}
