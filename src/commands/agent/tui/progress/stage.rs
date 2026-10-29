use crate::infra::github::{PrCiState, PrProgressInfo, PrState};
use serde::{Deserialize, Serialize};

use super::{ProgressMark, StageMarks};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::commands::agent::tui) struct CurrentPrStage {
    label: &'static str,
    waiting_for_user: bool,
}

impl CurrentPrStage {
    const fn new(label: &'static str, waiting_for_user: bool) -> Self {
        Self {
            label,
            waiting_for_user,
        }
    }

    pub(in crate::commands::agent::tui) const fn implementing() -> Self {
        Self::new("implementing", false)
    }

    pub(in crate::commands::agent::tui) fn label(self) -> &'static str {
        self.label
    }

    pub(in crate::commands::agent::tui) fn waiting_for_user(self) -> bool {
        self.waiting_for_user
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::commands::agent::tui) enum PrStageInfo {
    Open(PrCiState),
    Closed,
    Merged,
}

impl PrStageInfo {
    pub(in crate::commands::agent::tui) fn from_progress(progress: &PrProgressInfo) -> Self {
        match &progress.pull_request.state {
            PrState::Open => Self::Open(progress.ci_state),
            PrState::Closed => Self::Closed,
            PrState::Merged => Self::Merged,
        }
    }

    pub(in crate::commands::agent::tui) fn from_link_state(state: Option<&str>) -> Self {
        match state.map(str::to_ascii_lowercase).as_deref() {
            Some("closed") => Self::Closed,
            Some("merged") => Self::Merged,
            _ => Self::Open(PrCiState::Unknown),
        }
    }
}

pub(in crate::commands::agent::tui) fn current_pr_stage(
    stages: StageMarks,
    pr_stage_info: Option<&PrStageInfo>,
) -> CurrentPrStage {
    use ProgressMark::{Done, User, Waiting, Working};

    if stages.merge == Done || matches!(pr_stage_info, Some(PrStageInfo::Merged)) {
        return CurrentPrStage::new("merged", false);
    }
    if matches!(pr_stage_info, Some(PrStageInfo::Closed)) {
        return CurrentPrStage::new("closed", false);
    }
    if stages.merge == User {
        return CurrentPrStage::new("merge", true);
    }
    if stages.pr_body == User {
        return CurrentPrStage::new("body", true);
    }
    if stages.diff_review == User {
        return CurrentPrStage::new("crit", true);
    }
    if matches!(stages.pr_body, Working | Waiting) {
        return CurrentPrStage::new("body", false);
    }
    if let Some(PrStageInfo::Open(ci_state)) = pr_stage_info {
        match ci_state {
            PrCiState::Waiting | PrCiState::InProgress => {
                return CurrentPrStage::new("CI running", false);
            }
            PrCiState::Unknown => return CurrentPrStage::new("CI", false),
            PrCiState::Successful => {}
        }
    }
    if matches!(stages.diff_review, Working | Waiting) {
        return CurrentPrStage::new("crit", false);
    }
    if matches!(stages.ci, Working | Waiting) {
        return CurrentPrStage::new("CI running", false);
    }
    if matches!(stages.merge, Working | Waiting) {
        return CurrentPrStage::new("merge", false);
    }
    CurrentPrStage::implementing()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::implementing(
        StageMarks {
            implementation: ProgressMark::Working,
            diff_review: ProgressMark::NotStarted,
            pr_body: ProgressMark::NotStarted,
            ci: ProgressMark::NotStarted,
            merge: ProgressMark::NotStarted,
        },
        None,
        "implementing",
        false,
    )]
    #[case::open_pr_with_unknown_ci(
        StageMarks {
            implementation: ProgressMark::Done,
            diff_review: ProgressMark::Done,
            pr_body: ProgressMark::Done,
            ci: ProgressMark::NotStarted,
            merge: ProgressMark::NotStarted,
        },
        Some(PrStageInfo::Open(PrCiState::Unknown)),
        "CI",
        false,
    )]
    #[case::closed_pr_with_stale_review_activity(
        StageMarks {
            implementation: ProgressMark::Done,
            diff_review: ProgressMark::Working,
            pr_body: ProgressMark::NotStarted,
            ci: ProgressMark::NotStarted,
            merge: ProgressMark::NotStarted,
        },
        Some(PrStageInfo::Closed),
        "closed",
        false,
    )]
    #[case::crit_waiting_for_user(
        StageMarks {
            implementation: ProgressMark::Done,
            diff_review: ProgressMark::User,
            pr_body: ProgressMark::NotStarted,
            ci: ProgressMark::NotStarted,
            merge: ProgressMark::NotStarted,
        },
        None,
        "crit",
        true,
    )]
    #[case::crit_waiting_for_user_with_running_ci(
        StageMarks {
            implementation: ProgressMark::Done,
            diff_review: ProgressMark::User,
            pr_body: ProgressMark::Done,
            ci: ProgressMark::Working,
            merge: ProgressMark::Waiting,
        },
        Some(PrStageInfo::Open(PrCiState::InProgress)),
        "crit",
        true,
    )]
    #[case::body_waiting_for_user(
        StageMarks {
            implementation: ProgressMark::Done,
            diff_review: ProgressMark::Done,
            pr_body: ProgressMark::User,
            ci: ProgressMark::NotStarted,
            merge: ProgressMark::NotStarted,
        },
        None,
        "body",
        true,
    )]
    #[case::ci_running(
        StageMarks {
            implementation: ProgressMark::Done,
            diff_review: ProgressMark::Done,
            pr_body: ProgressMark::Done,
            ci: ProgressMark::Working,
            merge: ProgressMark::Waiting,
        },
        Some(PrStageInfo::Open(PrCiState::InProgress)),
        "CI running",
        false,
    )]
    #[case::merge_waiting_for_user(
        StageMarks {
            implementation: ProgressMark::Done,
            diff_review: ProgressMark::Done,
            pr_body: ProgressMark::Done,
            ci: ProgressMark::Done,
            merge: ProgressMark::User,
        },
        Some(PrStageInfo::Open(PrCiState::Successful)),
        "merge",
        true,
    )]
    #[case::merged(
        StageMarks {
            implementation: ProgressMark::Done,
            diff_review: ProgressMark::Done,
            pr_body: ProgressMark::Done,
            ci: ProgressMark::Done,
            merge: ProgressMark::Done,
        },
        Some(PrStageInfo::Merged),
        "merged",
        false,
    )]
    fn current_pr_stage_uses_one_plain_language_stage(
        #[case] marks: StageMarks,
        #[case] pr_stage_info: Option<PrStageInfo>,
        #[case] label: &str,
        #[case] waiting_for_user: bool,
    ) {
        let stage = current_pr_stage(marks, pr_stage_info.as_ref());

        assert_eq!(
            (stage.label(), stage.waiting_for_user()),
            (label, waiting_for_user)
        );
    }
}
