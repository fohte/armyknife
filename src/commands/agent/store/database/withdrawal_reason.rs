use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WithdrawalReason {
    SessionEnded,
    WorktreeCleanup,
    Manual,
}

impl WithdrawalReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SessionEnded => "session_ended",
            Self::WorktreeCleanup => "worktree_cleanup",
            Self::Manual => "manual",
        }
    }

    pub(crate) fn parse(value: Option<String>) -> Option<Self> {
        match value.as_deref() {
            Some("session_ended") => Some(Self::SessionEnded),
            Some("worktree_cleanup") => Some(Self::WorktreeCleanup),
            Some("manual") => Some(Self::Manual),
            Some(_) | None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::WithdrawalReason;

    #[rstest]
    #[case::legacy_null(None)]
    #[case::unknown_reason(Some("future_reason".to_string()))]
    fn unknown_and_legacy_reasons_are_treated_as_absent(#[case] value: Option<String>) {
        assert_eq!(WithdrawalReason::parse(value), None);
    }
}
