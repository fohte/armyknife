//! WM-specific git utilities.
//!
//! Re-exports common git functions and provides wm-specific helpers.

// Re-export common git functions for use by wm subcommands
pub use crate::infra::git::{
    branch_exists, get_main_branch, get_merge_status, get_repo_root, local_branch_exists,
    remote_branch_exists,
};

/// Normalize a branch name to a worktree directory name.
/// - Removes the given branch_prefix
/// - Replaces slashes with dashes
pub fn branch_to_worktree_name(branch: &str, branch_prefix: &str) -> String {
    let name_no_prefix = branch.strip_prefix(branch_prefix).unwrap_or(branch);
    name_no_prefix.replace('/', "-")
}

/// Default branch prefix for tests.
#[cfg(test)]
pub const BRANCH_PREFIX: &str = "fohte/";

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::simple("feature-branch", BRANCH_PREFIX, "feature-branch")]
    #[case::with_prefix("fohte/feature-branch", BRANCH_PREFIX, "feature-branch")]
    #[case::with_slash("feature/branch", BRANCH_PREFIX, "feature-branch")]
    #[case::with_prefix_and_slash("fohte/feature/branch", BRANCH_PREFIX, "feature-branch")]
    #[case::nested_slash("feature/sub/branch", BRANCH_PREFIX, "feature-sub-branch")]
    #[case::custom_prefix("user/feature-branch", "user/", "feature-branch")]
    #[case::custom_prefix_with_slash("user/feature/sub", "user/", "feature-sub")]
    #[case::no_match("feature-branch", "other/", "feature-branch")]
    #[case::empty_prefix("fohte/feature", "", "fohte-feature")]
    fn test_branch_to_worktree_name(
        #[case] branch: &str,
        #[case] prefix: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(branch_to_worktree_name(branch, prefix), expected);
    }
}
