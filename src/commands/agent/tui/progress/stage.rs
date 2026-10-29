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

pub(in crate::commands::agent::tui) fn current_pr_stage(stages: StageMarks) -> CurrentPrStage {
    use ProgressMark::{Done, User, Waiting, Working};

    if stages.merge == Done {
        return CurrentPrStage::new("merged", false);
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
    if matches!(stages.diff_review, Working | Waiting) {
        return CurrentPrStage::new("crit", false);
    }
    if matches!(stages.pr_body, Working | Waiting) {
        return CurrentPrStage::new("body", false);
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
        "implementing",
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
        "merged",
        false,
    )]
    fn current_pr_stage_uses_one_plain_language_stage(
        #[case] marks: StageMarks,
        #[case] label: &str,
        #[case] waiting_for_user: bool,
    ) {
        let stage = current_pr_stage(marks);

        assert_eq!(
            (stage.label(), stage.waiting_for_user()),
            (label, waiting_for_user)
        );
    }
}
