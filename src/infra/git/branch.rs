//! Branch operations.

use git2::{BranchType, Repository};

use super::repo::{get_main_branch, open_repo};

/// Check if a branch exists (local or remote)
pub fn branch_exists(branch: &str) -> bool {
    local_branch_exists(branch) || remote_branch_exists(branch)
}

/// Check if a local branch exists
pub fn local_branch_exists(branch: &str) -> bool {
    let Ok(repo) = open_repo() else {
        return false;
    };
    repo.find_branch(branch, BranchType::Local).is_ok()
}

/// Check if a remote branch exists
pub fn remote_branch_exists(branch: &str) -> bool {
    let Ok(repo) = open_repo() else {
        return false;
    };
    let remote_branch = format!("origin/{branch}");
    repo.find_branch(&remote_branch, BranchType::Remote).is_ok()
}

/// Find the base branch for PR creation.
///
/// Priority:
/// 1. If `origin/main` exists locally, return "main"
/// 2. If `origin/master` exists locally, return "master"
/// 3. Fallback to GitHub API to get the repository's default branch
///
/// This avoids unnecessary API calls when the base branch can be determined locally.
pub async fn find_base_branch<C: crate::infra::github::RepoClient>(
    owner: &str,
    repo_name: &str,
    client: &C,
) -> String {
    find_base_branch_impl(owner, repo_name, get_main_branch().ok(), client).await
}

/// Internal implementation that accepts optional local branch for testability.
async fn find_base_branch_impl<C: crate::infra::github::RepoClient>(
    owner: &str,
    repo_name: &str,
    local_branch: Option<String>,
    client: &C,
) -> String {
    // Try to use local git info first
    if let Some(branch) = local_branch {
        return branch;
    }

    // Fallback to GitHub API
    if let Ok(default_branch) = client.get_default_branch(owner, repo_name).await {
        return default_branch;
    }

    // Ultimate fallback
    "main".to_string()
}

/// Check if `branch` is an ancestor of `base` (equivalent to `git merge-base --is-ancestor`)
pub fn check_is_ancestor(branch: &str, base: &str) -> Option<bool> {
    let repo = open_repo().ok()?;
    check_is_ancestor_in_repo(&repo, branch, base)
}

/// Check if `branch` is an ancestor of `base` in a specific repository
fn check_is_ancestor_in_repo(repo: &Repository, branch: &str, base: &str) -> Option<bool> {
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

/// Check if a branch is merged (via PR or git merge-base)
pub async fn get_merge_status(branch_name: &str) -> MergeStatus {
    use super::github::get_owner_repo;
    use super::repo::get_main_branch;
    use crate::infra::github::{OctocrabClient, PrClient, PrState};

    // First, check PR status via GitHub API
    if let Some((owner, repo)) = get_owner_repo()
        && let Ok(client) = OctocrabClient::get()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::github::RepoClient;

    /// Mock RepoClient for testing find_base_branch_with_client
    struct MockRepoClient {
        default_branch: Option<String>,
    }

    impl MockRepoClient {
        fn with_default_branch(branch: &str) -> Self {
            Self {
                default_branch: Some(branch.to_string()),
            }
        }

        fn failing() -> Self {
            Self {
                default_branch: None,
            }
        }
    }

    #[async_trait::async_trait]
    impl RepoClient for MockRepoClient {
        async fn is_repo_private(
            &self,
            _owner: &str,
            _repo: &str,
        ) -> crate::infra::github::error::Result<bool> {
            Ok(false)
        }

        async fn get_default_branch(
            &self,
            _owner: &str,
            _repo: &str,
        ) -> crate::infra::github::error::Result<String> {
            self.default_branch.clone().ok_or_else(|| {
                crate::infra::github::GitHubError::TokenError("mock error".to_string()).into()
            })
        }
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

    #[tokio::test]
    async fn test_find_base_branch_uses_local_branch_when_provided() {
        let client = MockRepoClient::with_default_branch("develop");
        let result =
            find_base_branch_impl("owner", "repo", Some("master".to_string()), &client).await;
        // Local branch takes priority over GitHub API
        assert_eq!(result, "master");
    }

    #[tokio::test]
    async fn test_find_base_branch_uses_github_api_when_no_local_branch() {
        let client = MockRepoClient::with_default_branch("develop");
        let result = find_base_branch_impl("owner", "repo", None, &client).await;
        // Falls back to GitHub API
        assert_eq!(result, "develop");
    }

    #[tokio::test]
    async fn test_find_base_branch_fallback_to_main_when_api_fails() {
        let client = MockRepoClient::failing();
        let result = find_base_branch_impl("owner", "repo", None, &client).await;
        // Falls back to "main" when API call fails
        assert_eq!(result, "main");
    }
}
