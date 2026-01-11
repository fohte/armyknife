use clap::Args;
use std::io::{self, Write};
use std::process::Command;

use super::error::{Result, WmError};
use super::git::{get_merge_status, get_repo_root, local_branch_exists};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CleanArgs {
    /// Show what would be deleted without actually deleting
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

struct WorktreeInfo {
    path: String,
    branch: String,
    reason: String,
}

pub fn run(args: &CleanArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    run_inner(args)?;
    Ok(())
}

fn run_inner(args: &CleanArgs) -> Result<()> {
    git_fetch_prune()?;

    let repo_root = get_repo_root()?;
    let (to_delete, to_skip) = collect_worktrees(&repo_root)?;

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
    delete_worktrees(&to_delete)?;

    Ok(())
}

/// Run `git fetch -p` to prune stale remote-tracking references
fn git_fetch_prune() -> Result<()> {
    let status = Command::new("git")
        .args(["fetch", "-p"])
        .status()
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    if !status.success() {
        return Err(WmError::CommandFailed("git fetch failed".into()));
    }

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
fn delete_worktrees(worktrees: &[WorktreeInfo]) -> Result<()> {
    for wt in worktrees {
        if delete_single_worktree(wt)? {
            delete_branch_if_exists(&wt.branch)?;
        }
    }

    println!();
    println!("Done. Deleted {} worktree(s).", worktrees.len());

    Ok(())
}

/// Delete a single worktree. Returns true if successful.
fn delete_single_worktree(wt: &WorktreeInfo) -> Result<bool> {
    let has_submodules = std::path::Path::new(&wt.path).join(".gitmodules").exists();

    let mut args = vec!["worktree", "remove"];
    if has_submodules {
        args.push("--force");
    }
    args.push(&wt.path);

    let status = Command::new("git")
        .args(&args)
        .status()
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    if status.success() {
        println!("Deleted: {} ({})", wt.path, wt.reason);
        Ok(true)
    } else {
        eprintln!("Failed to delete: {}", wt.path);
        Ok(false)
    }
}

/// Delete a branch if it exists locally
fn delete_branch_if_exists(branch: &str) -> Result<()> {
    if branch.is_empty() || !local_branch_exists(branch) {
        return Ok(());
    }

    let status = Command::new("git")
        .args(["branch", "-D", branch])
        .status()
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    if status.success() {
        println!("  Branch deleted: {branch}");
    }

    Ok(())
}

/// Collect all worktrees and categorize them by merge status
fn collect_worktrees(repo_root: &str) -> Result<(Vec<WorktreeInfo>, Vec<WorktreeInfo>)> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        return Err(WmError::CommandFailed(
            "git worktree list --porcelain failed".into(),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut to_delete = Vec::new();
    let mut to_skip = Vec::new();

    let mut current_path: Option<String> = None;
    let mut current_branch: Option<String> = None;

    // Closure to process a worktree entry and categorize by merge status
    let mut process_entry = |path: String, branch: String| {
        if path == repo_root {
            return;
        }
        let merge_status = get_merge_status(&branch);
        let wt = WorktreeInfo {
            path,
            branch,
            reason: merge_status.reason().to_string(),
        };
        if merge_status.is_merged() {
            to_delete.push(wt);
        } else {
            to_skip.push(wt);
        }
    };

    for line in stdout.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current_path = Some(path.to_string());
            current_branch = None;
        } else if let Some(branch_ref) = line.strip_prefix("branch ") {
            // Remove refs/heads/ prefix
            let branch = branch_ref.strip_prefix("refs/heads/").unwrap_or(branch_ref);
            current_branch = Some(branch.to_string());
        } else if line.is_empty() {
            // End of worktree entry
            if let (Some(path), Some(branch)) = (current_path.take(), current_branch.take()) {
                process_entry(path, branch);
            }
        }
    }

    // Handle the last entry if there's no trailing newline
    if let (Some(path), Some(branch)) = (current_path, current_branch) {
        process_entry(path, branch);
    }

    Ok((to_delete, to_skip))
}
