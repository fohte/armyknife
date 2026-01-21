use git2::{Repository, Signature};
use std::path::PathBuf;

/// A temporary git repository for testing.
pub struct TestRepo {
    dir: tempfile::TempDir,
}

impl TestRepo {
    /// Create a new test repository with an initial commit.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");

        // Initialize git repo using git2
        let repo = Repository::init(dir.path()).expect("Failed to init repo");

        // Create initial commit
        let sig = Signature::now("Test User", "test@example.com").expect("Failed to create sig");
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .expect("Failed to create initial commit");

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
    pub fn open(&self) -> Repository {
        Repository::open(self.path()).expect("Failed to open repo")
    }

    /// Create a worktree with the given branch name.
    pub fn create_worktree(&self, branch_name: &str) {
        let worktree_path = self.worktree_path(branch_name);

        // Create .worktrees directory if it doesn't exist
        let worktrees_dir = self.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).expect("Failed to create .worktrees dir");

        let repo = self.open();

        // Create worktree using git2
        repo.worktree(branch_name, &worktree_path, None)
            .expect("Failed to create worktree");
    }

    /// Get the path to a worktree.
    pub fn worktree_path(&self, branch_name: &str) -> PathBuf {
        self.path().join(".worktrees").join(branch_name)
    }
}
