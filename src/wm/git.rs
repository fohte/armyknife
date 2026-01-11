use serde::Deserialize;
use std::process::Command;

use super::error::{Result, WmError};

/// Branch prefix for new branches created by `wm new`
pub const BRANCH_PREFIX: &str = "fohte/";

/// Get the main worktree root (the first entry in `git worktree list`).
/// This is always the main repository, regardless of which worktree we're in.
/// For bare repositories, this is the bare repo directory.
/// For regular repositories, this is the main working tree root.
pub fn get_repo_root() -> Result<String> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        return Err(WmError::NotInGitRepo);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // The first "worktree <path>" line is always the main worktree
    for line in stdout.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            return Ok(path.to_string());
        }
    }

    Err(WmError::NotInGitRepo)
}

/// Get the main branch name (main or master)
pub fn get_main_branch() -> Result<String> {
    // Check for origin/main first
    let main = Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            "refs/remotes/origin/main",
        ])
        .status()
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    if main.success() {
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
    Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if a remote branch exists
pub fn remote_branch_exists(branch: &str) -> bool {
    Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/remotes/origin/{branch}"),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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

#[derive(Deserialize)]
struct PrInfo {
    state: String,
    url: String,
}

/// Check if a branch is merged (via PR or git merge-base)
pub fn get_merge_status(branch_name: &str) -> MergeStatus {
    // First, check PR status via gh
    if let Some(pr_info) = Command::new("gh")
        .args(["pr", "view", branch_name, "--json", "state,url"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice::<PrInfo>(&o.stdout).ok())
    {
        match pr_info.state.as_str() {
            "MERGED" => {
                return MergeStatus::Merged {
                    reason: format!("PR {} merged", pr_info.url),
                };
            }
            "OPEN" => {
                return MergeStatus::NotMerged {
                    reason: format!("PR {} is open", pr_info.url),
                };
            }
            "CLOSED" => {
                return MergeStatus::NotMerged {
                    reason: format!("PR {} is closed (not merged)", pr_info.url),
                };
            }
            _ => {}
        }
    }

    // Fallback: check using git merge-base
    let main_branch = get_main_branch().unwrap_or_else(|_| "main".to_string());
    let base_branch = format!("origin/{main_branch}");

    let merge_base = Command::new("git")
        .args(["merge-base", "--is-ancestor", branch_name, &base_branch])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if merge_base {
        return MergeStatus::Merged {
            reason: format!("ancestor of {base_branch}"),
        };
    }

    MergeStatus::NotMerged {
        reason: "not merged (no PR found, not ancestor of base branch)".to_string(),
    }
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

    #[test]
    fn test_pr_info_deserialization() {
        let json = r#"{"state": "MERGED", "url": "https://github.com/fohte/armyknife/pull/1"}"#;
        let pr_info: PrInfo = serde_json::from_str(json).unwrap();
        assert_eq!(pr_info.state, "MERGED");
        assert_eq!(pr_info.url, "https://github.com/fohte/armyknife/pull/1");
    }
}
