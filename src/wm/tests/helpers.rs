use std::path::{Path, PathBuf};
use std::process::Command;

/// A temporary git repository for testing.
pub struct TestRepo {
    dir: tempfile::TempDir,
}

impl TestRepo {
    /// Create a new test repository with an initial commit.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");

        // Initialize git repo
        let status = Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .status()
            .expect("Failed to run git init");
        assert!(status.success(), "git init failed");

        // Configure git user for commits
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(dir.path())
            .status()
            .expect("Failed to configure git email");

        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(dir.path())
            .status()
            .expect("Failed to configure git name");

        // Create initial commit
        let status = Command::new("git")
            .args(["commit", "--allow-empty", "-m", "Initial commit"])
            .current_dir(dir.path())
            .status()
            .expect("Failed to run git commit");
        assert!(status.success(), "git commit failed");

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

    /// Create a worktree with the given branch name.
    pub fn create_worktree(&self, branch_name: &str) {
        let worktree_path = self.worktree_path(branch_name);

        // Create .worktrees directory if it doesn't exist
        let worktrees_dir = self.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).expect("Failed to create .worktrees dir");

        let status = Command::new("git")
            .args([
                "worktree",
                "add",
                worktree_path.to_str().unwrap(),
                "-b",
                branch_name,
            ])
            .current_dir(self.path())
            .status()
            .expect("Failed to run git worktree add");
        assert!(status.success(), "git worktree add failed");
    }

    /// Get the path to a worktree.
    pub fn worktree_path(&self, branch_name: &str) -> PathBuf {
        self.path().join(".worktrees").join(branch_name)
    }

    /// Run a closure in the repository directory.
    pub fn run_in_dir<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let _guard = WorkingDirGuard::change(&self.path());
        f()
    }

    /// Run a closure in a worktree directory.
    pub fn run_in_worktree<F, R>(&self, branch_name: &str, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let worktree_path = self.worktree_path(branch_name);
        let _guard = WorkingDirGuard::change(&worktree_path);
        f()
    }
}

/// RAII guard that changes the current working directory and restores it on drop.
pub struct WorkingDirGuard {
    original: PathBuf,
}

impl WorkingDirGuard {
    pub fn change(path: &Path) -> Self {
        let original = std::env::current_dir().expect("Failed to get current dir");
        std::env::set_current_dir(path).expect("Failed to change directory");
        Self { original }
    }
}

impl Drop for WorkingDirGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}
