use git2::{BranchType, Repository};
use std::path::Path;

use super::error::{Result, WmError};
use crate::github::{GitHubClient, OctocrabClient, PrState};

/// Branch prefix for new branches created by `wm new`
pub const BRANCH_PREFIX: &str = "fohte/";

/// Open a git repository from a path.
fn open_repo(path: &Path) -> Result<Repository> {
    Repository::open_ext(
        path,
        git2::RepositoryOpenFlags::empty(),
        std::iter::empty::<&Path>(),
    )
    .map_err(|_| WmError::NotInGitRepo)
}

/// Get the main worktree root (the first entry in `git worktree list`).
/// This is always the main repository, regardless of which worktree we're in.
/// For bare repositories, this is the bare repo directory.
/// For regular repositories, this is the main working tree root.
pub fn get_repo_root() -> Result<String> {
    let cwd = std::env::current_dir().map_err(|e| WmError::CommandFailed(e.to_string()))?;
    get_repo_root_in(&cwd)
}

/// Get the main worktree root from the specified directory.
pub fn get_repo_root_in(cwd: &Path) -> Result<String> {
    let repo = open_repo(cwd)?;

    let path = if repo.is_worktree() {
        // For worktrees, commondir() points to the main repo's .git
        // The main worktree's workdir is the parent of commondir
        let commondir = repo.commondir();
        commondir.parent().ok_or(WmError::NotInGitRepo)?
    } else {
        // For the main repo, workdir() gives us the working directory
        repo.workdir().ok_or(WmError::NotInGitRepo)?
    };

    // Normalize path: remove trailing slash for consistency
    let path_str = path.to_string_lossy();
    Ok(path_str.trim_end_matches('/').to_string())
}

/// Get the main branch name (main or master)
pub fn get_main_branch() -> Result<String> {
    let repo = Repository::open_from_env().map_err(|_| WmError::NotInGitRepo)?;
    get_main_branch_for_repo(&repo)
}

/// Get the main branch name for a specific repository
fn get_main_branch_for_repo(repo: &Repository) -> Result<String> {
    // Check for origin/main first
    if repo.find_branch("origin/main", BranchType::Remote).is_ok() {
        return Ok("main".to_string());
    }

    // Fall back to master
    Ok("master".to_string())
}

/// Check if a branch exists (local or remote)
pub fn branch_exists(branch: &str) -> bool {
    local_branch_exists(branch) || remote_branch_exists(branch)
}

/// Check if a local branch exists
pub fn local_branch_exists(branch: &str) -> bool {
    let Ok(repo) = Repository::open_from_env() else {
        return false;
    };
    repo.find_branch(branch, BranchType::Local).is_ok()
}

/// Check if a remote branch exists
pub fn remote_branch_exists(branch: &str) -> bool {
    let Ok(repo) = Repository::open_from_env() else {
        return false;
    };
    let remote_branch = format!("origin/{branch}");
    repo.find_branch(&remote_branch, BranchType::Remote).is_ok()
}

#[derive(Debug, Clone)]
pub enum MergeStatus {
    Merged { reason: String },
    NotMerged { reason: String },
}

impl MergeStatus {
    pub fn is_merged(&self) -> bool {
        matches!(self, MergeStatus::Merged { .. })
    }

    pub fn reason(&self) -> &str {
        match self {
            MergeStatus::Merged { reason } | MergeStatus::NotMerged { reason } => reason,
        }
    }
}

/// Get owner and repo from git remote URL.
fn get_owner_repo() -> Option<(String, String)> {
    let repo = Repository::open_from_env().ok()?;
    let remote = repo.find_remote("origin").ok()?;
    let url = remote.url()?;

    // Parse GitHub URL formats:
    // - https://github.com/owner/repo.git
    // - git@github.com:owner/repo.git
    let path = if let Some(rest) = url.strip_prefix("https://github.com/") {
        rest
    } else if let Some(rest) = url.strip_prefix("git@github.com:") {
        rest
    } else {
        return None;
    };

    let path = path.strip_suffix(".git").unwrap_or(path);
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

/// Check if a branch is merged (via PR or git merge-base)
pub async fn get_merge_status(branch_name: &str) -> MergeStatus {
    // First, check PR status via GitHub API
    if let Some((owner, repo)) = get_owner_repo()
        && let Ok(client) = OctocrabClient::new()
        && let Ok(Some(pr_info)) = client.get_pr_for_branch(&owner, &repo, branch_name).await
    {
        match pr_info.state {
            PrState::Merged => {
                return MergeStatus::Merged {
                    reason: format!("PR {} merged", pr_info.url),
                };
            }
            PrState::Open => {
                return MergeStatus::NotMerged {
                    reason: format!("PR {} is open", pr_info.url),
                };
            }
            PrState::Closed => {
                return MergeStatus::NotMerged {
                    reason: format!("PR {} is closed (not merged)", pr_info.url),
                };
            }
        }
    }

    // Fallback: check using git2 merge-base
    let main_branch = get_main_branch().unwrap_or_else(|_| "main".to_string());
    let base_branch = format!("origin/{main_branch}");

    if let Some(true) = check_is_ancestor(branch_name, &base_branch) {
        return MergeStatus::Merged {
            reason: format!("ancestor of {base_branch}"),
        };
    }

    MergeStatus::NotMerged {
        reason: "not merged (no PR found, not ancestor of base branch)".to_string(),
    }
}

/// Check if `branch` is an ancestor of `base` (equivalent to `git merge-base --is-ancestor`)
fn check_is_ancestor(branch: &str, base: &str) -> Option<bool> {
    let repo = Repository::open_from_env().ok()?;

    // Resolve branch to commit
    let branch_oid = repo
        .revparse_single(branch)
        .ok()?
        .peel_to_commit()
        .ok()?
        .id();

    // Resolve base to commit
    let base_oid = repo.revparse_single(base).ok()?.peel_to_commit().ok()?.id();

    // `--is-ancestor A B` checks if A is ancestor of B
    // This means B descends from A
    // graph_descendant_of(descendant, ancestor) returns true if descendant is from ancestor
    repo.graph_descendant_of(base_oid, branch_oid).ok()
}

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

    #[test]
    fn test_merge_status_is_merged() {
        let merged = MergeStatus::Merged {
            reason: "test".to_string(),
        };
        let not_merged = MergeStatus::NotMerged {
            reason: "test".to_string(),
        };

        assert!(merged.is_merged());
        assert!(!not_merged.is_merged());
    }

    #[test]
    fn test_merge_status_reason() {
        let merged = MergeStatus::Merged {
            reason: "PR merged".to_string(),
        };
        let not_merged = MergeStatus::NotMerged {
            reason: "PR is open".to_string(),
        };

        assert_eq!(merged.reason(), "PR merged");
        assert_eq!(not_merged.reason(), "PR is open");
    }
}
