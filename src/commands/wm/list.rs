use std::path::PathBuf;

use clap::Args;
use git2::Repository;

use super::error::{Result, WmError};
use super::worktree::{get_main_worktree_info, get_main_worktree_path, list_linked_worktrees};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ListArgs {}

pub fn run(_args: &ListArgs) -> Result<()> {
    let repo = Repository::open_from_env().map_err(|_| WmError::NotInGitRepo)?;
    let entries = list_worktrees(&repo)?;
    print!("{}", format_worktree_list(&entries));
    if !entries.is_empty() {
        println!();
    }
    Ok(())
}

/// Information about a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: String,
    pub commit: String,
}

/// Format multiple worktree entries for display.
/// Uses dynamic width based on the longest path.
pub fn format_worktree_list(entries: &[WorktreeInfo]) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let max_path_len = entries
        .iter()
        .map(|e| e.path.display().to_string().len())
        .max()
        .unwrap_or(0);

    entries
        .iter()
        .map(|e| {
            format!(
                "{:<width$} {} [{}]",
                e.path.display(),
                e.commit,
                e.branch,
                width = max_path_len
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// List all worktrees (main + linked) for a repository.
pub fn list_worktrees(repo: &Repository) -> Result<Vec<WorktreeInfo>> {
    let mut entries = Vec::new();

    // Add main worktree
    let main_path = get_main_worktree_path(repo)?;
    let (main_branch, main_commit) = get_main_worktree_info(repo);
    entries.push(WorktreeInfo {
        path: main_path,
        branch: main_branch,
        commit: main_commit,
    });

    // Add linked worktrees
    for wt in list_linked_worktrees(repo)? {
        entries.push(WorktreeInfo {
            path: wt.path,
            branch: wt.branch,
            commit: wt.commit,
        });
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::TestRepo;

    #[test]
    fn list_worktrees_returns_main_repo() {
        let repo = TestRepo::new();
        let entries = list_worktrees(&repo.open()).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, repo.path());
        assert_eq!(entries[0].branch, "master");
        assert_eq!(entries[0].commit.len(), 7);
    }

    #[test]
    fn list_worktrees_includes_linked_worktrees() {
        let repo = TestRepo::new();
        repo.create_worktree("feature-branch");

        let entries = list_worktrees(&repo.open()).unwrap();

        assert_eq!(entries.len(), 2);
        // Main repo
        assert_eq!(entries[0].path, repo.path());
        // Linked worktree
        assert_eq!(entries[1].path, repo.worktree_path("feature-branch"));
        assert_eq!(entries[1].branch, "feature-branch");
    }

    #[test]
    fn list_worktrees_from_worktree_lists_all() {
        let repo = TestRepo::new();
        repo.create_worktree("feature-branch");

        // Open from the worktree instead of main repo
        let wt_repo = Repository::open(repo.worktree_path("feature-branch")).unwrap();
        let entries = list_worktrees(&wt_repo).unwrap();

        // Should still list both worktrees
        assert_eq!(entries.len(), 2);
    }

    // Output format spec tests

    use indoc::indoc;
    use rstest::rstest;

    fn entry(path: &str, commit: &str, branch: &str) -> WorktreeInfo {
        WorktreeInfo {
            path: PathBuf::from(path),
            commit: commit.to_string(),
            branch: branch.to_string(),
        }
    }

    #[rstest]
    #[case::single_entry(
        vec![entry("/repo", "abc1234", "main")],
        "/repo abc1234 [main]"
    )]
    #[case::two_entries_same_length(
        vec![
            entry("/repo1", "1111111", "main"),
            entry("/repo2", "2222222", "dev"),
        ],
        indoc! {"
            /repo1 1111111 [main]
            /repo2 2222222 [dev]"}
    )]
    #[case::dynamic_width_alignment(
        vec![
            entry("/short", "1111111", "main"),
            entry("/much-longer-path", "2222222", "feature"),
        ],
        indoc! {"
            /short            1111111 [main]
            /much-longer-path 2222222 [feature]"}
    )]
    #[case::three_entries_with_varying_lengths(
        vec![
            entry("/a", "aaa1111", "main"),
            entry("/medium-path", "bbb2222", "develop"),
            entry("/x", "ccc3333", "feature"),
        ],
        indoc! {"
            /a           aaa1111 [main]
            /medium-path bbb2222 [develop]
            /x           ccc3333 [feature]"}
    )]
    fn format_worktree_list_produces_expected_output(
        #[case] entries: Vec<WorktreeInfo>,
        #[case] expected: &str,
    ) {
        assert_eq!(format_worktree_list(&entries), expected);
    }

    #[test]
    fn format_worktree_list_empty_returns_empty_string() {
        let entries: Vec<WorktreeInfo> = vec![];
        assert_eq!(format_worktree_list(&entries), "");
    }
}
