use chrono::{DateTime, Utc};

use crate::commands::agent::store::WithdrawalReason;

pub(super) fn session_ended_at(
    reason: Option<WithdrawalReason>,
    resolved_at: Option<&DateTime<Utc>>,
) -> Option<String> {
    if reason == Some(WithdrawalReason::SessionEnded) {
        resolved_at.map(DateTime::to_rfc3339)
    } else {
        None
    }
}
