use std::path::{Path, PathBuf};
use std::process::Command;

/// A temporary git repository for testing.
pub struct TestRepo {
    dir: tempfile::TempDir,
}

impl TestRepo {
    /// Create a git Command with isolated config (ignores global/system settings).
    fn git_command(dir: &Path) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(dir);
        // Ignore global/system git config to ensure tests are isolated from
        // local settings (e.g., GPG signing, aliases, hooks).
        cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
        cmd.env("GIT_CONFIG_SYSTEM", "/dev/null");
        cmd
    }

    /// Create a new test repository with an initial commit.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");

        // Initialize git repo
        let status = Self::git_command(dir.path())
            .args(["init"])
            .status()
            .expect("Failed to run git init");
        assert!(status.success(), "git init failed");

        // Configure git user for commits
        Self::git_command(dir.path())
            .args(["config", "user.email", "test@example.com"])
            .status()
            .expect("Failed to configure git email");

        Self::git_command(dir.path())
            .args(["config", "user.name", "Test User"])
            .status()
            .expect("Failed to configure git name");

        // Create initial commit
        let status = Self::git_command(dir.path())
            .args(["commit", "--allow-empty", "-m", "Initial commit"])
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

        let status = Self::git_command(&self.path())
            .args([
                "worktree",
                "add",
                worktree_path.to_str().unwrap(),
                "-b",
                branch_name,
            ])
            .status()
            .expect("Failed to run git worktree add");
        assert!(status.success(), "git worktree add failed");
    }

    /// Get the path to a worktree.
    pub fn worktree_path(&self, branch_name: &str) -> PathBuf {
        self.path().join(".worktrees").join(branch_name)
    }
}
