use anyhow::Result;
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

    pub(crate) fn parse(value: Option<String>) -> Result<Option<Self>> {
        match value.as_deref() {
            Some("session_ended") => Ok(Some(Self::SessionEnded)),
            Some("worktree_cleanup") => Ok(Some(Self::WorktreeCleanup)),
            Some("manual") => Ok(Some(Self::Manual)),
            Some(value) => anyhow::bail!("unknown withdrawal reason: {value}"),
            None => Ok(None),
        }
    }
}
