//! Git operations using git2 (libgit2).
//!
//! This module provides a unified interface for git operations without
//! spawning external git processes.

mod branch;
mod error;
mod github;
mod repo;
#[cfg(test)]
pub mod test_utils;

#[allow(unused_imports)] // MergeStatus is returned by get_merge_status
pub use branch::{
    MergeStatus, branch_exists, get_merge_status, local_branch_exists, remote_branch_exists,
};
pub use error::GitError;
pub use github::github_owner_and_repo;
pub use repo::{current_branch, fetch_with_prune, get_main_branch, get_repo_root, open_repo};

// Re-export for test utilities
#[cfg(test)]
pub use repo::{get_repo_root_in, open_repo_at};

#[cfg(test)]
pub use test_utils::TempRepo;
