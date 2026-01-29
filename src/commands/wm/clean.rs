use anyhow::Context;
use clap::Args;
use git2::Repository;
use std::io::{self, Write};

use super::error::{Result, WmError};
use super::git::get_merge_status;
use super::worktree::{
    LinkedWorktree, delete_branch_if_exists, delete_worktree, get_main_repo, list_linked_worktrees,
};
use crate::infra::git::fetch_with_prune;
use crate::infra::tmux;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CleanArgs {
    /// Show what would be deleted without actually deleting
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

/// Worktree with merge status and associated tmux windows for clean command.
struct CleanWorktreeInfo {
    wt: LinkedWorktree,
    reason: String,
    /// Tmux window IDs that are located in this worktree's path
    window_ids: Vec<String>,
}

pub async fn run(args: &CleanArgs) -> Result<()> {
    let repo = Repository::open_from_env().map_err(|_| WmError::NotInGitRepo)?;
    let main_repo = get_main_repo(&repo)?;

    fetch_with_prune(&main_repo).context("Failed to fetch from remote")?;

    let (to_delete, to_skip) = collect_worktrees(&main_repo).await?;

    display_worktrees_to_keep(&to_skip);

    if to_delete.is_empty() {
        println!("No merged worktrees to delete.");
        return Ok(());
    }

    display_worktrees_to_delete(&to_delete);

    if args.dry_run {
        println!();
        println!("(dry-run mode, no changes made)");
        return Ok(());
    }

    if !confirm_deletion() {
        println!("Cancelled.");
        return Ok(());
    }

    println!();
    delete_worktrees(&main_repo, &to_delete)?;

    Ok(())
}

/// Display worktrees that will be kept
fn display_worktrees_to_keep(worktrees: &[CleanWorktreeInfo]) {
    if worktrees.is_empty() {
        return;
    }

    println!("Worktrees to keep:");
    for info in worktrees {
        println!("  {} ({})", info.wt.path.display(), info.reason);
    }
    println!();
}

/// Display worktrees that will be deleted
fn display_worktrees_to_delete(worktrees: &[CleanWorktreeInfo]) {
    println!("Worktrees to delete:");
    for info in worktrees {
        println!("  {} ({})", info.wt.path.display(), info.reason);

        for window_id in &info.window_ids {
            println!("    -> tmux window: {window_id}");
        }
    }
}

/// Prompt user for confirmation
fn confirm_deletion() -> bool {
    println!();
    print!("Delete these worktrees? [y/N] ");
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    input.trim().eq_ignore_ascii_case("y")
}

/// Delete all worktrees and their branches
fn delete_worktrees(repo: &Repository, worktrees: &[CleanWorktreeInfo]) -> Result<()> {
    let mut deleted_count = 0;

    for info in worktrees {
        if delete_worktree(repo, &info.wt.name)? {
            println!("Deleted: {} ({})", info.wt.path.display(), info.reason);
            deleted_count += 1;

            if delete_branch_if_exists(repo, &info.wt.branch) {
                println!("  Branch deleted: {}", info.wt.branch);
            }

            // Close tmux windows that were in the deleted worktree
            for window_id in &info.window_ids {
                if tmux::kill_window(window_id).is_ok() {
                    println!("  Tmux window closed: {window_id}");
                }
            }
        }
    }

    println!();
    println!("Done. Deleted {deleted_count} worktree(s).");

    Ok(())
}

/// Collect all worktrees and categorize them by merge status
async fn collect_worktrees(
    repo: &Repository,
) -> Result<(Vec<CleanWorktreeInfo>, Vec<CleanWorktreeInfo>)> {
    let mut to_delete = Vec::new();
    let mut to_skip = Vec::new();

    for wt in list_linked_worktrees(repo)? {
        if wt.branch.is_empty() || wt.branch == "(unknown)" {
            continue;
        }

        let merge_status = get_merge_status(&wt.branch).await;
        // Collect tmux window IDs while the worktree path still exists
        let window_ids = tmux::get_window_ids_in_path(&wt.path.to_string_lossy());
        let info = CleanWorktreeInfo {
            wt,
            reason: merge_status.reason().to_string(),
            window_ids,
        };

        if merge_status.is_merged() {
            to_delete.push(info);
        } else {
            to_skip.push(info);
        }
    }

    Ok((to_delete, to_skip))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::TestRepo;
    use std::path::PathBuf;

    fn make_clean_info(name: &str, path: PathBuf, branch: &str, reason: &str) -> CleanWorktreeInfo {
        CleanWorktreeInfo {
            wt: LinkedWorktree {
                name: name.to_string(),
                path,
                branch: branch.to_string(),
                commit: "abc1234".to_string(),
            },
            reason: reason.to_string(),
            window_ids: Vec::new(),
        }
    }

    #[test]
    fn delete_worktrees_deletes_all_worktrees() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature-a");
        test_repo.create_worktree("feature-b");

        let repo = test_repo.open();

        let worktrees = vec![
            make_clean_info(
                "feature-a",
                test_repo.worktree_path("feature-a"),
                "feature-a",
                "merged",
            ),
            make_clean_info(
                "feature-b",
                test_repo.worktree_path("feature-b"),
                "feature-b",
                "merged",
            ),
        ];

        delete_worktrees(&repo, &worktrees).unwrap();

        // Verify worktrees are deleted
        assert!(repo.find_worktree("feature-a").is_err());
        assert!(repo.find_worktree("feature-b").is_err());
    }

    #[test]
    fn delete_worktrees_handles_empty_list() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let result = delete_worktrees(&repo, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn display_worktrees_to_keep_does_nothing_for_empty() {
        // Just ensure it doesn't panic
        display_worktrees_to_keep(&[]);
    }

    #[test]
    fn display_worktrees_to_delete_does_not_panic() {
        let test_repo = TestRepo::new();
        let worktrees = vec![make_clean_info(
            "feature",
            test_repo.path().join(".worktrees/feature"),
            "feature",
            "merged",
        )];

        // Just ensure it doesn't panic
        display_worktrees_to_delete(&worktrees);
    }
}
