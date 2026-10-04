//! Exit codes for review commands.

use super::HumanInTheLoopError;

/// The user did not approve the review (closed the editor without approving).
pub const NOT_APPROVED: i32 = 1;

/// The editor is already open for this file (lock exists).
pub const ALREADY_OPEN: i32 = 2;

/// The terminal emulator failed to launch (e.g., macOS asleep, Ghostty
/// initialization error). Callers can retry once the system wakes.
pub const TERMINAL_LAUNCH_FAILED: i32 = 3;

/// The tmux review pane disappeared before it signaled completion.
pub const REVIEW_PANE_CLOSED: i32 = 4;

/// Convert errors from `start_review` into dedicated process exit codes where
/// callers need to distinguish a failed terminal launch or a closed review pane.
///
/// This keeps the mapping in one place so each review command handler can
/// share the same behavior.
pub fn exit_on_review_failure<T>(result: super::Result<T>) -> anyhow::Result<T>
where
    anyhow::Error: From<HumanInTheLoopError>,
{
    match result {
        Ok(value) => Ok(value),
        Err(error) => match review_error_exit_code(&error) {
            Some(code) => {
                eprintln!("{error}");
                std::process::exit(code);
            }
            None => Err(error.into()),
        },
    }
}

fn review_error_exit_code(error: &HumanInTheLoopError) -> Option<i32> {
    match error {
        HumanInTheLoopError::TerminalLaunchFailed { .. } => Some(TERMINAL_LAUNCH_FAILED),
        HumanInTheLoopError::ReviewPaneClosed => Some(REVIEW_PANE_CLOSED),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[test]
    fn exit_on_review_failure_passes_through_ok() {
        let value: anyhow::Result<i32> = exit_on_review_failure(Ok(42));
        assert_eq!(value.expect("ok"), 42);
    }

    #[test]
    fn exit_on_review_failure_propagates_other_errors() {
        let input: super::super::Result<i32> = Err(HumanInTheLoopError::NotApproved);
        let err = exit_on_review_failure(input).expect_err("expected propagated error");
        assert_eq!(
            err.to_string(),
            "Not approved. Run 'review' and set 'submit: true'"
        );
    }

    #[rstest]
    #[case::terminal_launch_failed(
        HumanInTheLoopError::TerminalLaunchFailed { timeout_secs: 10 },
        Some(TERMINAL_LAUNCH_FAILED),
    )]
    #[case::review_pane_closed(HumanInTheLoopError::ReviewPaneClosed, Some(REVIEW_PANE_CLOSED))]
    #[case::other_errors_propagate(HumanInTheLoopError::NotApproved, None)]
    fn maps_only_expected_review_failures_to_exit_codes(
        #[case] error: HumanInTheLoopError,
        #[case] expected: Option<i32>,
    ) {
        assert_eq!(review_error_exit_code(&error), expected);
    }
}
