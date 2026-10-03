use std::path::{Path, PathBuf};

use crate::infra::git::{GitRepo, get_merge_status_for_repo};
use crate::shared::worktree::{find_worktree_name, get_main_repo, get_worktree_branch};

/// Worktree details needed by cleanup actions after merge status is resolved.
pub struct MergedWorktree {
    pub main_repo: GitRepo,
    pub branch: String,
    pub path: PathBuf,
}

/// Resolves a worktree's repository and branch when its PR has merged.
/// Returns `None` when the path is not a linked worktree or its branch is
/// unresolved.
pub async fn find_merged_worktree_at(path: &Path) -> Option<MergedWorktree> {
    let Ok(repo) = GitRepo::open_at(path) else {
        return None;
    };
    if !repo.is_worktree() {
        return None;
    }
    let Ok(main_repo) = get_main_repo(&repo) else {
        return None;
    };
    let worktree_root = repo.workdir().to_path_buf();
    let Ok(worktree_name) = find_worktree_name(&main_repo, &worktree_root.to_string_lossy()) else {
        return None;
    };
    let branch = get_worktree_branch(&main_repo, &worktree_name)?;

    get_merge_status_for_repo(&main_repo, &branch)
        .await
        .is_merged()
        .then_some(MergedWorktree {
            main_repo,
            branch,
            path: worktree_root,
        })
}
