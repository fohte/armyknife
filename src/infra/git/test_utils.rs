//! Test utilities for creating temporary git repositories.

use git2::Repository;
use std::path::PathBuf;
use tempfile::TempDir;

use super::repo::open_repo_at;

/// A temporary git repository for testing.
pub struct TempRepo {
    pub dir: TempDir,
}

impl TempRepo {
    /// Create a new temporary git repository with a GitHub-style origin remote.
    pub fn new(owner: &str, repo_name: &str, branch: &str) -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let repo = Repository::init(dir.path()).expect("init repo");

        // Create initial commit so HEAD exists
        {
            let sig = git2::Signature::now("Test", "test@example.com").unwrap();
            let tree_id = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
                .expect("create initial commit");
        }

        // Rename default branch if needed
        {
            let head = repo.head().expect("get head");
            let current_branch = head.shorthand().unwrap_or("master").to_string();
            drop(head); // Release borrow before renaming
            if current_branch != branch {
                let mut branch_ref = repo
                    .find_branch(&current_branch, git2::BranchType::Local)
                    .expect("find branch");
                branch_ref.rename(branch, true).expect("rename branch");
            }
        }

        // Set up origin remote with GitHub URL
        let url = format!("https://github.com/{owner}/{repo_name}.git");
        repo.remote("origin", &url).expect("set origin");

        Self { dir }
    }

    /// Get the path to the repository.
    pub fn path(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    /// Open the repository.
    pub fn open(&self) -> Repository {
        open_repo_at(self.dir.path()).expect("open temp repo")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::git::{current_branch, github_owner_and_repo};

    #[test]
    fn test_temp_repo_creates_valid_repo() {
        let temp = TempRepo::new("test-owner", "test-repo", "main");
        let repo = temp.open();

        let branch = current_branch(&repo).unwrap();
        assert_eq!(branch, "main");

        let (owner, repo_name) = github_owner_and_repo(&repo).unwrap();
        assert_eq!(owner, "test-owner");
        assert_eq!(repo_name, "test-repo");
    }
}
