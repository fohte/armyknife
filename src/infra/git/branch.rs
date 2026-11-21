//! Branch operations.

use anyhow::Context;

use super::cmd::run_git;
use super::repo::{GitRepo, get_main_branch_for_repo, open_repo};
use crate::infra::github::{PrInfo, PrState};

/// Check if a local branch exists
pub fn local_branch_exists(branch: &str) -> bool {
    let Ok(repo) = open_repo() else {
        return false;
    };
    repo.local_branch_exists(branch)
}

/// Find the base branch for PR creation via the GitHub API's default branch.
///
/// This intentionally does not consult the local git checkout (e.g. cwd's
/// `origin/main`/`origin/master`), since the target `owner`/`repo_name` may
/// differ from the repository checked out at cwd.
pub async fn find_base_branch<C: crate::infra::github::RepoClient>(
    owner: &str,
    repo_name: &str,
    client: &C,
) -> String {
    if let Ok(default_branch) = client.get_default_branch(owner, repo_name).await {
        return default_branch;
    }

    // Ultimate fallback
    "main".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeStatus {
    Merged { reason: String },
    Closed { reason: String },
    NotMerged { reason: String },
}

impl MergeStatus {
    pub fn is_merged(&self) -> bool {
        matches!(self, MergeStatus::Merged { .. })
    }

    /// Whether this worktree should be cleaned up (deleted).
    ///
    /// Returns true for both `Merged` and `Closed` statuses:
    /// - Merged: PR was merged, worktree is no longer needed
    /// - Closed: PR was closed without merging, worktree is no longer needed
    pub fn should_cleanup(&self) -> bool {
        matches!(
            self,
            MergeStatus::Merged { .. } | MergeStatus::Closed { .. }
        )
    }

    pub fn reason(&self) -> &str {
        match self {
            MergeStatus::Merged { reason }
            | MergeStatus::Closed { reason }
            | MergeStatus::NotMerged { reason } => reason,
        }
    }
}

/// Convert PR information into merge status.
///
/// Extracts the PR number from the URL and maps the PR state
/// to the corresponding merge status.
pub fn merge_status_from_pr(pr_info: &PrInfo) -> MergeStatus {
    // Fall back to pr_info.number if URL is empty or malformed
    let pr_number = pr_info
        .url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(|n| format!("#{n}"))
        .unwrap_or_else(|| format!("#{}", pr_info.number));

    match pr_info.state {
        PrState::Merged => MergeStatus::Merged {
            reason: format!("{pr_number} merged"),
        },
        PrState::Open => MergeStatus::NotMerged {
            reason: format!("{pr_number} open"),
        },
        PrState::Closed => MergeStatus::Closed {
            reason: format!("{pr_number} closed"),
        },
    }
}

/// Fallback merge status when no PR is found for a branch.
///
/// Always returns `NotMerged` to protect branches that may have
/// work-in-progress changes.
pub fn merge_status_from_git(_repo: &GitRepo, _branch_name: &str) -> MergeStatus {
    // When no PR is found, always treat the branch as not merged.
    // The git merge-base check (is-ancestor) gives false positives for branches
    // that have no commits yet, since their HEAD is an ancestor of the main branch.
    // These branches may be work-in-progress, so it's safer to keep them.
    MergeStatus::NotMerged {
        reason: "No PR found".to_string(),
    }
}

/// Whether a local branch contains commits that are not reachable from the
/// repository's configured main branch on `origin`.
pub(crate) fn branch_has_commits_ahead_of_main(
    repo: &GitRepo,
    branch_name: &str,
) -> anyhow::Result<bool> {
    let main_branch = get_main_branch_for_repo(repo)?;
    branch_has_commits_ahead_of_main_with(branch_name, &main_branch, |range| {
        run_git(repo.workdir(), ["rev-list", "--count", range])
    })
}

fn branch_has_commits_ahead_of_main_with(
    branch_name: &str,
    main_branch: &str,
    rev_list: impl FnOnce(&str) -> anyhow::Result<String>,
) -> anyhow::Result<bool> {
    let range = format!("refs/remotes/origin/{main_branch}..refs/heads/{branch_name}");
    let raw_count = rev_list(&range)?;
    let count = raw_count
        .parse::<usize>()
        .context("Failed to parse branch commit count")?;
    Ok(count > 0)
}

/// Check if a branch is merged (via PR or git merge-base).
///
/// Uses the repository from the current working directory.
/// For cross-repository operations, use [`get_merge_status_for_repo`] instead.
pub async fn get_merge_status(branch_name: &str) -> MergeStatus {
    let repo = open_repo().ok();
    get_merge_status_impl(repo.as_ref(), branch_name).await
}

/// Check if a branch is merged, using a specific repository.
///
/// Unlike [`get_merge_status`], this does not open a repository from the
/// current working directory. Use this in cross-repository operations
/// (e.g., `--all` mode) where the target repo differs from the CWD.
pub async fn get_merge_status_for_repo(repo: &GitRepo, branch_name: &str) -> MergeStatus {
    get_merge_status_impl(Some(repo), branch_name).await
}

async fn get_merge_status_impl(repo: Option<&GitRepo>, branch_name: &str) -> MergeStatus {
    use super::github::github_owner_and_repo;
    use crate::infra::github::{GitHubClient, PrClient};

    // First, check PR status via GitHub API
    if let Some(repo) = repo
        && let Ok((owner, repo_name)) = github_owner_and_repo(repo)
    {
        let client = tokio::task::spawn_blocking(GitHubClient::get)
            .await
            .ok()
            .and_then(std::result::Result::ok);
        if let Some(client) = client
            && let Ok(Some(pr_info)) = client
                .get_pr_for_branch(&owner, &repo_name, branch_name)
                .await
        {
            return merge_status_from_pr(&pr_info);
        }
    }

    // Fallback: protect branches without an associated PR.
    if let Some(repo) = repo {
        return merge_status_from_git(repo, branch_name);
    }

    MergeStatus::NotMerged {
        reason: "Not merged".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::github::RepoClient;
    use rstest::rstest;

    #[rstest]
    #[case::no_ahead_commits("0", false)]
    #[case::one_ahead_commit("1", true)]
    fn branch_ahead_check_counts_commits_from_the_selected_main_branch(
        #[case] raw_count: &str,
        #[case] expected: bool,
    ) {
        let mut range = None;
        let result = branch_has_commits_ahead_of_main_with("feature", "main", |value| {
            range = Some(value.to_string());
            Ok(raw_count.to_string())
        });

        assert_eq!(
            (result.unwrap(), range),
            (
                expected,
                Some("refs/remotes/origin/main..refs/heads/feature".to_string()),
            ),
        );
    }

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

    impl RepoClient for MockRepoClient {
        async fn repo_exists(
            &self,
            _owner: &str,
            _repo: &str,
        ) -> crate::infra::github::error::Result<bool> {
            Ok(self.default_branch.is_some())
        }

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
        let closed = MergeStatus::Closed {
            reason: "test".to_string(),
        };
        let not_merged = MergeStatus::NotMerged {
            reason: "test".to_string(),
        };

        assert!(merged.is_merged());
        assert!(!closed.is_merged());
        assert!(!not_merged.is_merged());
    }

    #[rstest]
    #[case::merged(MergeStatus::Merged { reason: "test".to_string() }, true)]
    #[case::closed(MergeStatus::Closed { reason: "test".to_string() }, true)]
    #[case::not_merged(MergeStatus::NotMerged { reason: "test".to_string() }, false)]
    fn test_merge_status_should_cleanup(#[case] status: MergeStatus, #[case] expected: bool) {
        assert_eq!(status.should_cleanup(), expected);
    }

    #[test]
    fn test_merge_status_reason() {
        let merged = MergeStatus::Merged {
            reason: "PR merged".to_string(),
        };
        let closed = MergeStatus::Closed {
            reason: "PR closed".to_string(),
        };
        let not_merged = MergeStatus::NotMerged {
            reason: "PR is open".to_string(),
        };

        assert_eq!(merged.reason(), "PR merged");
        assert_eq!(closed.reason(), "PR closed");
        assert_eq!(not_merged.reason(), "PR is open");
    }

    #[tokio::test]
    async fn test_find_base_branch_uses_github_api() {
        let client = MockRepoClient::with_default_branch("develop");
        let result = find_base_branch("owner", "repo", &client).await;
        assert_eq!(result, "develop");
    }

    #[tokio::test]
    async fn test_find_base_branch_fallback_to_main_when_api_fails() {
        let client = MockRepoClient::failing();
        let result = find_base_branch("owner", "repo", &client).await;
        // Falls back to "main" when API call fails
        assert_eq!(result, "main");
    }
}
