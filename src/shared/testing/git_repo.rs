use std::path::{Path, PathBuf};
use std::process::Command;

use crate::infra::git::GitRepo;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .status()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(status.success(), "git {args:?} failed");
}

/// A temporary git repository for testing.
pub struct TestRepo {
    dir: tempfile::TempDir,
}

impl TestRepo {
    /// Create a new test repository with an initial commit.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let path = dir.path();
        git(path, &["init", "-q"]);
        git(
            path,
            &["commit", "--allow-empty", "-q", "-m", "Initial commit"],
        );
        Self { dir }
    }

    /// Get the canonicalized path to the repository.
    /// This resolves symlinks (e.g., /var -> /private/var on macOS).
    pub fn path(&self) -> PathBuf {
        self.dir
            .path()
            .canonicalize()
            .expect("Failed to canonicalize path")
    }

    /// Open the repository.
    pub fn open(&self) -> GitRepo {
        crate::infra::git::open_repo_at(&self.path()).expect("Failed to open repo")
    }

    /// Create a worktree with the given branch name.
    pub fn create_worktree(&self, branch_name: &str) {
        let worktree_path = self.worktree_path(branch_name);

        let worktrees_dir = self.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).expect("Failed to create .worktrees dir");

        git(
            &self.path(),
            &[
                "worktree",
                "add",
                "-b",
                branch_name,
                worktree_path.to_str().expect("utf8 path"),
            ],
        );
    }

    /// Get the path to a worktree.
    pub fn worktree_path(&self, branch_name: &str) -> PathBuf {
        self.path().join(".worktrees").join(branch_name)
    }
}
