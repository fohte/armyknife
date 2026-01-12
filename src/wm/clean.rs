use clap::Args;
use git2::{BranchType, Repository, WorktreePruneOptions};
use std::io::{self, Write};

use super::error::{Result, WmError};
use super::git::{get_merge_status, get_repo_root, local_branch_exists};
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

    // Get the main repo (if we're in a worktree, get the parent)
    let main_repo = if repo.is_worktree() {
        let commondir = repo.commondir();
        Repository::open(commondir.parent().ok_or(WmError::NotInGitRepo)?)
            .map_err(|_| WmError::NotInGitRepo)?
    } else {
        repo
    };

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
    for wt in worktrees {
        if delete_single_worktree(repo, wt)? {
            delete_branch_if_exists(repo, &wt.branch)?;
        }
    }

    println!();
    println!("Done. Deleted {} worktree(s).", worktrees.len());

    Ok(())
}

/// Delete a single worktree. Returns true if successful.
fn delete_single_worktree(repo: &Repository, wt: &WorktreeInfo) -> Result<bool> {
    let worktree = match repo.find_worktree(&wt.name) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Failed to find worktree {}: {}", wt.path, e);
            return Ok(false);
        }
    };

    let mut prune_opts = WorktreePruneOptions::new();
    prune_opts.valid(true).working_tree(true);

    match worktree.prune(Some(&mut prune_opts)) {
        Ok(()) => {
            println!("Deleted: {} ({})", wt.path, wt.reason);
            Ok(true)
        }
        Err(e) => {
            eprintln!("Failed to delete {}: {}", wt.path, e);
            Ok(false)
        }
    }
}

/// Delete a branch if it exists locally
fn delete_branch_if_exists(repo: &Repository, branch: &str) -> Result<()> {
    if branch.is_empty() || !local_branch_exists(branch) {
        return Ok(());
    }

    if let Ok(mut branch_ref) = repo.find_branch(branch, BranchType::Local)
        && branch_ref.delete().is_ok()
    {
        println!("  Branch deleted: {branch}");
    }

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

/// Get the branch name associated with a worktree
fn get_worktree_branch(repo: &Repository, worktree_name: &str) -> Option<String> {
    let worktree = repo.find_worktree(worktree_name).ok()?;
    let wt_repo = Repository::open_from_worktree(&worktree).ok()?;
    let head = wt_repo.head().ok()?;
    head.shorthand().map(|s| s.to_string())
}
