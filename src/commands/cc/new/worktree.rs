use anyhow::{Context, Result};
use std::path::Path;

use crate::infra::git::GitRepo;
use crate::infra::git::cmd::run_git;

/// Mode for creating a worktree
pub(super) enum WorktreeAddMode<'a> {
    /// Checkout an existing local branch
    LocalBranch { branch: &'a str },
    /// Create a tracking branch from remote
    TrackRemote { branch: &'a str },
    /// Create a new branch from base
    NewBranch { branch: &'a str, base: &'a str },
    /// Force create/reset a branch from base
    ForceNewBranch { branch: &'a str, base: &'a str },
}

/// Run `git worktree add` with the specified mode.
pub(super) fn git_worktree_add(
    repo: &GitRepo,
    worktree_dir: &Path,
    mode: WorktreeAddMode,
) -> Result<()> {
    let path = worktree_dir.to_str().context("Invalid worktree path")?;

    match mode {
        WorktreeAddMode::LocalBranch { branch } => {
            run_git(repo.workdir(), ["worktree", "add", path, branch])
                .context("Failed to add worktree")?;
        }
        WorktreeAddMode::TrackRemote { branch } => {
            let remote_name = format!("origin/{branch}");
            run_git(
                repo.workdir(),
                [
                    "worktree",
                    "add",
                    "--track",
                    "-b",
                    branch,
                    path,
                    &remote_name,
                ],
            )
            .context("Failed to add worktree")?;
        }
        WorktreeAddMode::NewBranch { branch, base } => {
            run_git(
                repo.workdir(),
                ["worktree", "add", "-b", branch, path, base],
            )
            .context("Failed to add worktree")?;
        }
        WorktreeAddMode::ForceNewBranch { branch, base } => {
            // `--force -B` overrides both "branch already used by worktree"
            // and "destination path exists" checks so the reset succeeds even
            // when the branch is checked out elsewhere.
            run_git(
                repo.workdir(),
                ["worktree", "add", "--force", "-B", branch, path, base],
            )
            .context("Failed to add worktree")?;
        }
    }

    Ok(())
}

/// Check if a branch exists (local or remote) in the given repository.
pub(super) fn repo_branch_exists(repo: &GitRepo, branch: &str) -> bool {
    repo.local_branch_exists(branch) || repo.remote_branch_exists(&format!("origin/{branch}"))
}

/// Add a worktree for an existing branch (local or remote)
pub(super) fn add_worktree_for_branch(
    repo: &GitRepo,
    worktree_dir: &Path,
    branch: &str,
) -> Result<()> {
    if repo.local_branch_exists(branch) {
        git_worktree_add(repo, worktree_dir, WorktreeAddMode::LocalBranch { branch })
    } else if repo.remote_branch_exists(&format!("origin/{branch}")) {
        git_worktree_add(repo, worktree_dir, WorktreeAddMode::TrackRemote { branch })
    } else {
        // Fallback: use as-is (should not normally happen)
        git_worktree_add(repo, worktree_dir, WorktreeAddMode::LocalBranch { branch })
    }
}

/// How to roll back the branch associated with a worktree after a
/// post-worktree-create hook failure.
pub(super) enum BranchRollback {
    /// Branch was created in this invocation; delete it.
    Delete,
    /// Branch pre-existed and was not modified; leave it alone.
    Keep,
    /// Branch was force-reset to a new base; restore its previous tip.
    RestoreTip(String),
}

pub(super) fn rollback_worktree(
    repo: &GitRepo,
    worktree_name: &str,
    branch: &str,
    branch_rollback: &BranchRollback,
) {
    eprintln!("post-worktree-create hook failed; rolling back worktree '{worktree_name}'");

    let removed = match crate::commands::wm::worktree::delete_worktree(repo, worktree_name) {
        Ok(true) => true,
        Ok(false) => {
            eprintln!(
                "warning: worktree '{worktree_name}' could not be removed. \
                 Run `a wm delete` or remove it manually before re-running `a wm new`."
            );
            false
        }
        Err(e) => {
            eprintln!(
                "warning: failed to remove worktree '{worktree_name}': {e}. \
                 Run `a wm delete` or remove it manually before re-running `a wm new`."
            );
            false
        }
    };

    // Skip branch mutation while the worktree still references it; git would
    // refuse the delete/update anyway, and leaving state intact lets the user
    // retry after manual worktree cleanup.
    if !removed {
        if let BranchRollback::RestoreTip(tip) = branch_rollback {
            eprintln!(
                "warning: branch '{branch}' was force-reset and cannot be restored \
                 while the worktree remains. After removing the worktree, run \
                 `git update-ref refs/heads/{branch} {tip}`."
            );
        }
        return;
    }

    match branch_rollback {
        BranchRollback::Delete => {
            if crate::commands::wm::worktree::delete_branch_if_exists(repo, branch) {
                eprintln!("Deleted branch '{branch}'");
            }
        }
        BranchRollback::Keep => {}
        BranchRollback::RestoreTip(tip) => {
            match run_git(
                repo.workdir(),
                ["update-ref", &format!("refs/heads/{branch}"), tip],
            ) {
                Ok(_) => eprintln!("Restored branch '{branch}' to {tip}"),
                Err(e) => eprintln!(
                    "warning: failed to restore branch '{branch}' to {tip}: {e}. \
                     Recover with `git update-ref refs/heads/{branch} {tip}` or `git reflog`."
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::TestRepo;
    use rstest::{fixture, rstest};
    use std::path::PathBuf;
    use std::process::Command;

    fn git_in(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn git_worktree_add_creates_worktree_with_new_branch() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let worktrees_dir = test_repo.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();

        let worktree_dir = worktrees_dir.join("test-branch");
        git_worktree_add(
            &repo,
            &worktree_dir,
            WorktreeAddMode::NewBranch {
                branch: "test-branch",
                base: "HEAD",
            },
        )
        .unwrap();

        assert!(worktree_dir.exists());
        assert!(repo.local_branch_exists("test-branch"));
    }

    #[test]
    fn git_worktree_add_with_local_branch() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        git_in(&test_repo.path(), &["branch", "existing-branch"]);

        let worktrees_dir = test_repo.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();

        let worktree_dir = worktrees_dir.join("existing-branch");
        git_worktree_add(
            &repo,
            &worktree_dir,
            WorktreeAddMode::LocalBranch {
                branch: "existing-branch",
            },
        )
        .unwrap();

        assert!(worktree_dir.exists());
    }

    #[test]
    fn git_worktree_add_force_resets_existing_branch() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        git_in(&test_repo.path(), &["branch", "force-test"]);
        git_in(
            &test_repo.path(),
            &["commit", "--allow-empty", "-q", "-m", "Second commit"],
        );

        let worktrees_dir = test_repo.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();

        let worktree_dir = worktrees_dir.join("force-test");
        git_worktree_add(
            &repo,
            &worktree_dir,
            WorktreeAddMode::ForceNewBranch {
                branch: "force-test",
                base: "HEAD",
            },
        )
        .unwrap();

        assert!(worktree_dir.exists());
    }

    #[rstest]
    #[case::local_exists("existing-local", true)]
    #[case::remote_exists("existing-remote", true)]
    #[case::nonexistent("nonexistent", false)]
    fn repo_branch_exists_checks_local_and_remote(#[case] branch: &str, #[case] expected: bool) {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        git_in(&test_repo.path(), &["branch", "existing-local"]);
        let head = run_git(&test_repo.path(), ["rev-parse", "HEAD"]).unwrap();
        run_git(
            &test_repo.path(),
            ["update-ref", "refs/remotes/origin/existing-remote", &head],
        )
        .unwrap();

        assert_eq!(repo_branch_exists(&repo, branch), expected);
    }

    #[rstest]
    fn add_worktree_for_branch_uses_local_branch() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();
        git_in(&test_repo.path(), &["branch", "local-branch"]);

        let worktrees_dir = test_repo.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();
        let worktree_dir = worktrees_dir.join("local-branch");

        add_worktree_for_branch(&repo, &worktree_dir, "local-branch").unwrap();
        assert!(worktree_dir.exists());
    }

    #[rstest]
    fn add_worktree_for_branch_tracks_remote_branch() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();
        let repo_path = test_repo.path();

        // Simulate a remote-tracking branch to avoid spawning a real remote,
        // providing the minimum refs and config required for git worktree --track to resolve.
        let head = run_git(&repo_path, ["rev-parse", "HEAD"]).unwrap();
        run_git(
            &repo_path,
            ["update-ref", "refs/remotes/origin/remote-branch", &head],
        )
        .unwrap();
        run_git(&repo_path, ["config", "remote.origin.url", "."]).unwrap();
        run_git(
            &repo_path,
            [
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/remotes/origin/*",
            ],
        )
        .unwrap();

        let worktrees_dir = repo_path.join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();
        let worktree_dir = worktrees_dir.join("remote-branch");

        add_worktree_for_branch(&repo, &worktree_dir, "remote-branch").unwrap();
        assert!(worktree_dir.exists());
        assert!(repo.local_branch_exists("remote-branch"));
    }

    struct RollbackEnv {
        // hold the TempDir so files survive the test
        _test_repo: TestRepo,
        repo: GitRepo,
        worktree_dir: PathBuf,
    }

    #[fixture]
    fn rollback_env() -> RollbackEnv {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();
        let worktrees_dir = test_repo.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();
        let worktree_dir = worktrees_dir.join("rollback-branch");
        RollbackEnv {
            _test_repo: test_repo,
            repo,
            worktree_dir,
        }
    }

    #[rstest]
    fn rollback_worktree_deletes_created_branch(rollback_env: RollbackEnv) {
        git_worktree_add(
            &rollback_env.repo,
            &rollback_env.worktree_dir,
            WorktreeAddMode::NewBranch {
                branch: "rollback-branch",
                base: "HEAD",
            },
        )
        .unwrap();

        rollback_worktree(
            &rollback_env.repo,
            "rollback-branch",
            "rollback-branch",
            &BranchRollback::Delete,
        );

        assert!(!rollback_env.worktree_dir.exists());
        assert!(!rollback_env.repo.local_branch_exists("rollback-branch"));
    }

    #[rstest]
    fn rollback_worktree_keeps_preexisting_branch(rollback_env: RollbackEnv) {
        git_in(rollback_env.repo.workdir(), &["branch", "rollback-branch"]);

        add_worktree_for_branch(
            &rollback_env.repo,
            &rollback_env.worktree_dir,
            "rollback-branch",
        )
        .unwrap();

        rollback_worktree(
            &rollback_env.repo,
            "rollback-branch",
            "rollback-branch",
            &BranchRollback::Keep,
        );

        assert!(!rollback_env.worktree_dir.exists());
        assert!(rollback_env.repo.local_branch_exists("rollback-branch"));
    }

    #[rstest]
    fn rollback_worktree_restores_force_reset_branch_tip(rollback_env: RollbackEnv) {
        let workdir = rollback_env.repo.workdir().to_path_buf();
        let original_tip = run_git(&workdir, ["rev-parse", "HEAD"]).unwrap();

        git_worktree_add(
            &rollback_env.repo,
            &rollback_env.worktree_dir,
            WorktreeAddMode::NewBranch {
                branch: "rollback-branch",
                base: "HEAD",
            },
        )
        .unwrap();

        let output = Command::new("git")
            .arg("-C")
            .arg(&workdir)
            .args([
                "commit-tree",
                "-m",
                "advance",
                &format!("{original_tip}^{{tree}}"),
            ])
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .unwrap();
        assert!(output.status.success(), "commit-tree failed");
        let advanced_tip = String::from_utf8(output.stdout).unwrap().trim().to_string();
        run_git(
            &workdir,
            ["update-ref", "refs/heads/rollback-branch", &advanced_tip],
        )
        .unwrap();
        assert_ne!(original_tip, advanced_tip);

        rollback_worktree(
            &rollback_env.repo,
            "rollback-branch",
            "rollback-branch",
            &BranchRollback::RestoreTip(original_tip.clone()),
        );

        assert!(!rollback_env.worktree_dir.exists());
        assert!(rollback_env.repo.local_branch_exists("rollback-branch"));
        assert_eq!(
            run_git(&workdir, ["rev-parse", "rollback-branch"]).unwrap(),
            original_tip,
        );
    }
}
