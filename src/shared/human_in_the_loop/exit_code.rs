//! Exit codes for review commands.

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
