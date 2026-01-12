use clap::Args;
use git2::Repository;
use std::io::{self, Write};

use super::error::{Result, WmError};
use super::git::{get_merge_status, get_repo_root};
use super::worktree::{
    delete_branch_if_exists, delete_worktree, get_main_repo, get_worktree_branch,
};
use crate::git::fetch_with_prune;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CleanArgs {
    /// Show what would be deleted without actually deleting
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

struct WorktreeInfo {
    name: String,
    path: String,
    branch: String,
    reason: String,
}

#[tokio::main]
pub async fn run(args: &CleanArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    run_inner(args).await?;
    Ok(())
}

async fn run_inner(args: &CleanArgs) -> Result<()> {
    let repo = Repository::open_from_env().map_err(|_| WmError::NotInGitRepo)?;
    let main_repo = get_main_repo(&repo)?;

    fetch_with_prune(&main_repo).map_err(|e| WmError::CommandFailed(e.to_string()))?;

    let repo_root = get_repo_root()?;
    let (to_delete, to_skip) = collect_worktrees(&main_repo, &repo_root).await?;

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
fn display_worktrees_to_keep(worktrees: &[WorktreeInfo]) {
    if worktrees.is_empty() {
        return;
    }

    println!("Worktrees to keep:");
    for wt in worktrees {
        println!("  {} ({})", wt.path, wt.reason);
    }
    println!();
}

/// Display worktrees that will be deleted
fn display_worktrees_to_delete(worktrees: &[WorktreeInfo]) {
    println!("Worktrees to delete:");
    for wt in worktrees {
        println!("  {} ({})", wt.path, wt.reason);
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
fn delete_worktrees(repo: &Repository, worktrees: &[WorktreeInfo]) -> Result<()> {
    let mut deleted_count = 0;

    for wt in worktrees {
        if delete_worktree(repo, &wt.name)? {
            println!("Deleted: {} ({})", wt.path, wt.reason);
            deleted_count += 1;

            if delete_branch_if_exists(repo, &wt.branch) {
                println!("  Branch deleted: {}", wt.branch);
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
    repo_root: &str,
) -> Result<(Vec<WorktreeInfo>, Vec<WorktreeInfo>)> {
    let worktrees = repo
        .worktrees()
        .map_err(|e| WmError::CommandFailed(e.message().to_string()))?;

    let mut to_delete = Vec::new();
    let mut to_skip = Vec::new();

    for name in worktrees.iter().flatten() {
        let wt = match repo.find_worktree(name) {
            Ok(w) => w,
            Err(_) => continue,
        };

        let wt_path = wt.path().to_string_lossy().to_string();

        // Skip the main worktree
        if wt_path.trim_end_matches('/') == repo_root.trim_end_matches('/') {
            continue;
        }

        // Get the branch name from the worktree
        let branch = get_worktree_branch(repo, name).unwrap_or_default();

        if branch.is_empty() {
            continue;
        }

        let merge_status = get_merge_status(&branch).await;
        let wt_info = WorktreeInfo {
            name: name.to_string(),
            path: wt_path,
            branch,
            reason: merge_status.reason().to_string(),
        };

        if merge_status.is_merged() {
            to_delete.push(wt_info);
        } else {
            to_skip.push(wt_info);
        }
    }

    Ok((to_delete, to_skip))
}
