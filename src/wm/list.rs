use std::path::PathBuf;

use clap::Args;
use git2::Repository;

use super::error::{Result, WmError};

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

impl WorktreeInfo {
    /// Format a single worktree entry for display.
    /// Format: `{path:<50} {commit} [{branch}]`
    pub fn format_line(&self) -> String {
        format!(
            "{:<50} {} [{}]",
            self.path.display(),
            self.commit,
            self.branch
        )
    }
}

/// Format multiple worktree entries for display.
pub fn format_worktree_list(entries: &[WorktreeInfo]) -> String {
    entries
        .iter()
        .map(|e| e.format_line())
        .collect::<Vec<_>>()
        .join("\n")
}

/// List all worktrees (main + linked) for a repository.
pub fn list_worktrees(repo: &Repository) -> Result<Vec<WorktreeInfo>> {
    let mut entries = Vec::new();

    // Get the main worktree path
    let main_path = if repo.is_worktree() {
        repo.commondir()
            .parent()
            .ok_or(WmError::NotInGitRepo)?
            .to_path_buf()
    } else {
        repo.workdir().ok_or(WmError::NotInGitRepo)?.to_path_buf()
    };

    // Add main worktree
    let head = repo.head().ok();
    let main_branch = head
        .as_ref()
        .and_then(|h| h.shorthand())
        .unwrap_or("(unknown)")
        .to_string();
    let main_commit = head
        .as_ref()
        .and_then(|h| h.peel_to_commit().ok())
        .map(|c| c.id().to_string())
        .map(|s| s[..7].to_string())
        .unwrap_or_else(|| "(none)".to_string());

    entries.push(WorktreeInfo {
        path: main_path,
        branch: main_branch,
        commit: main_commit,
    });

    // List linked worktrees
    let worktrees = repo
        .worktrees()
        .map_err(|e| WmError::CommandFailed(e.message().to_string()))?;
    for name in worktrees.iter().flatten() {
        if let Ok(wt) = repo.find_worktree(name) {
            let wt_path = wt.path().to_path_buf();
            // Open the worktree repository to get its HEAD
            if let Ok(wt_repo) = Repository::open(&wt_path) {
                let wt_head = wt_repo.head().ok();
                let branch = wt_head
                    .as_ref()
                    .and_then(|h| h.shorthand())
                    .unwrap_or("(unknown)")
                    .to_string();
                let commit = wt_head
                    .as_ref()
                    .and_then(|h| h.peel_to_commit().ok())
                    .map(|c| c.id().to_string())
                    .map(|s| s[..7].to_string())
                    .unwrap_or_else(|| "(none)".to_string());
                entries.push(WorktreeInfo {
                    path: wt_path,
                    branch,
                    commit,
                });
            }
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestRepo;
    use rstest::rstest;

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

    #[rstest]
    #[case::short_path(
        "/tmp/repo",
        "abc1234",
        "main",
        "/tmp/repo                                          abc1234 [main]"
    )]
    #[case::long_path(
        "/home/user/projects/very-long-repository-name-here",
        "def5678",
        "feature",
        "/home/user/projects/very-long-repository-name-here def5678 [feature]"
    )]
    #[case::exact_50_chars(
        "/home/user/projects/exactly-fifty-chars-path-here",
        "1234567",
        "dev",
        "/home/user/projects/exactly-fifty-chars-path-here  1234567 [dev]"
    )]
    fn format_line_produces_expected_output(
        #[case] path: &str,
        #[case] commit: &str,
        #[case] branch: &str,
        #[case] expected: &str,
    ) {
        let info = WorktreeInfo {
            path: PathBuf::from(path),
            branch: branch.to_string(),
            commit: commit.to_string(),
        };
        assert_eq!(info.format_line(), expected);
    }

    #[test]
    fn format_line_pads_path_to_50_chars() {
        let info = WorktreeInfo {
            path: PathBuf::from("/short"),
            branch: "main".to_string(),
            commit: "abc1234".to_string(),
        };
        let line = info.format_line();

        // Path should be padded to 50 chars, then space, commit, space, [branch]
        assert!(line.starts_with("/short"));
        // Find where the commit hash starts (after 50 chars of path area)
        let parts: Vec<&str> = line.splitn(2, "abc1234").collect();
        assert_eq!(parts[0].len(), 51); // 50 chars path + 1 space
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
