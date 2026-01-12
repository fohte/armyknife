use std::path::PathBuf;

use clap::Args;
use git2::Repository;

use super::error::{Result, WmError};
use super::worktree::{get_main_worktree_info, get_main_worktree_path, list_linked_worktrees};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ListArgs {}

pub fn run(_args: &ListArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
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
    use crate::testing::TestRepo;

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

    #[test]
    fn format_worktree_list_uses_dynamic_width() {
        let entries = vec![
            WorktreeInfo {
                path: PathBuf::from("/short"),
                branch: "main".to_string(),
                commit: "1111111".to_string(),
            },
            WorktreeInfo {
                path: PathBuf::from("/much-longer-path"),
                branch: "feature".to_string(),
                commit: "2222222".to_string(),
            },
        ];

        let output = format_worktree_list(&entries);
        let lines: Vec<&str> = output.lines().collect();

        // Both lines should align commits at the same column
        let commit_pos_1 = lines[0].find("1111111").unwrap();
        let commit_pos_2 = lines[1].find("2222222").unwrap();
        assert_eq!(commit_pos_1, commit_pos_2);

        // Width should match the longest path (17 chars) + 1 space
        assert_eq!(commit_pos_1, 18);
    }

    #[test]
    fn format_worktree_list_single_entry_no_padding() {
        let entries = vec![WorktreeInfo {
            path: PathBuf::from("/repo"),
            branch: "main".to_string(),
            commit: "abc1234".to_string(),
        }];

        let output = format_worktree_list(&entries);
        assert_eq!(output, "/repo abc1234 [main]");
    }

    #[test]
    fn format_worktree_list_format_structure() {
        let entries = vec![WorktreeInfo {
            path: PathBuf::from("/path/to/repo"),
            branch: "feature-branch".to_string(),
            commit: "abc1234".to_string(),
        }];

        let output = format_worktree_list(&entries);
        // Format: "{path} {commit} [{branch}]"
        assert!(output.starts_with("/path/to/repo"));
        assert!(output.contains("abc1234"));
        assert!(output.ends_with("[feature-branch]"));
    }

    #[test]
    fn format_worktree_list_joins_with_newlines() {
        let entries = vec![
            WorktreeInfo {
                path: PathBuf::from("/repo1"),
                branch: "main".to_string(),
                commit: "1111111".to_string(),
            },
            WorktreeInfo {
                path: PathBuf::from("/repo2"),
                branch: "feature".to_string(),
                commit: "2222222".to_string(),
            },
        ];

        let output = format_worktree_list(&entries);
        let lines: Vec<&str> = output.lines().collect();

        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("/repo1"));
        assert!(lines[0].contains("[main]"));
        assert!(lines[1].contains("/repo2"));
        assert!(lines[1].contains("[feature]"));
    }

    #[test]
    fn format_worktree_list_empty_returns_empty_string() {
        let entries: Vec<WorktreeInfo> = vec![];
        assert_eq!(format_worktree_list(&entries), "");
    }
}
