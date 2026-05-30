//! Test utilities for creating temporary git repositories.

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

use super::repo::{GitRepo, open_repo_at};

fn git(dir: &std::path::Path, args: &[&str]) {
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

/// A temporary git repository for testing.
pub struct TempRepo {
    pub dir: TempDir,
}

impl TempRepo {
    /// Create a new temporary git repository with a GitHub-style origin remote.
    pub fn new(owner: &str, repo_name: &str, branch: &str) -> Self {
        // Some sandboxes (srt) lazily create TMPDIR; ensure it exists before
        // TempDir probes the parent.
        let tmp_root = std::env::temp_dir();
        std::fs::create_dir_all(&tmp_root)
            .unwrap_or_else(|e| panic!("create temp root {}: {e}", tmp_root.display()));
        let dir = TempDir::new()
            .unwrap_or_else(|e| panic!("create temp dir under {}: {e}", tmp_root.display()));

        let path = dir.path();
        git(path, &["init", "-q", "-b", branch]);
        git(
            path,
            &["commit", "--allow-empty", "-q", "-m", "Initial commit"],
        );

        let url = format!("https://github.com/{owner}/{repo_name}.git");
        git(path, &["remote", "add", "origin", &url]);

        Self { dir }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    pub fn open(&self) -> GitRepo {
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
