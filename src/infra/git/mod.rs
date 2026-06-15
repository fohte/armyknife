//! Git operations via the `git` CLI.
//!
//! Wraps `git` subcommands with typed helpers ([`GitRepo`] etc.) so the rest
//! of the codebase operates on a single boundary instead of shelling out ad
//! hoc and so we can drop the libgit2 C dependency.

mod branch;
pub(crate) mod cmd;
mod error;
mod fetch_lock;
mod github;
mod repo;
#[cfg(test)]
pub mod test_utils;

pub use branch::{
    MergeStatus, find_base_branch, get_merge_status, get_merge_status_for_repo,
    local_branch_exists, merge_status_from_git, merge_status_from_pr,
};
pub use error::GitError;
pub use github::{get_owner_repo, github_owner_and_repo};
pub use repo::{
    GitRepo, current_branch, fetch_with_prune, get_main_branch_for_repo, get_repo_owner_and_name,
    get_repo_root, get_repo_root_in, open_repo, open_repo_at, parse_repo,
};
