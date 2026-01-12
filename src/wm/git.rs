//! WM-specific git utilities.
//!
//! Re-exports common git functions and provides wm-specific helpers.

// Re-export common git functions for use by wm subcommands
pub use crate::git::{
    branch_exists, get_main_branch, get_merge_status, get_repo_root, local_branch_exists,
    remote_branch_exists,
};

/// Branch prefix for new branches created by `wm new`
pub const BRANCH_PREFIX: &str = "fohte/";

/// Normalize a branch name to a worktree directory name
/// - Removes BRANCH_PREFIX
/// - Replaces slashes with dashes
pub fn branch_to_worktree_name(branch: &str) -> String {
    let name_no_prefix = branch.strip_prefix(BRANCH_PREFIX).unwrap_or(branch);
    name_no_prefix.replace('/', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::simple("feature-branch", "feature-branch")]
    #[case::with_prefix("fohte/feature-branch", "feature-branch")]
    #[case::with_slash("feature/branch", "feature-branch")]
    #[case::with_prefix_and_slash("fohte/feature/branch", "feature-branch")]
    #[case::nested_slash("feature/sub/branch", "feature-sub-branch")]
    fn test_branch_to_worktree_name(#[case] branch: &str, #[case] expected: &str) {
        assert_eq!(branch_to_worktree_name(branch), expected);
    }
}
