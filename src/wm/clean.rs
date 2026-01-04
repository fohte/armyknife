use clap::Args;
use std::io::{self, Write};
use std::process::Command;

use super::common::{Result, WmError, get_merge_status, get_repo_root, local_branch_exists};

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
    // Fetch with prune
    let fetch_status = Command::new("git")
        .args(["fetch", "-p"])
        .status()
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    if !fetch_status.success() {
        return Err(WmError::CommandFailed("git fetch failed".into()));
    }

    let repo_root = get_repo_root()?;

    // Collect worktrees with their merge status
    let (to_delete, to_skip) = collect_worktrees(&repo_root)?;

    // Display skipped worktrees
    if !to_skip.is_empty() {
        println!("Worktrees to keep:");
        for wt in &to_skip {
            println!("  {} ({})", wt.path, wt.reason);
        }
        println!();
    }

    // Display worktrees to delete
    if to_delete.is_empty() {
        println!("No merged worktrees to delete.");
        return Ok(());
    }

    println!("Worktrees to delete:");
    for wt in &to_delete {
        println!("  {} ({})", wt.path, wt.reason);
    }

    if args.dry_run {
        println!();
        println!("(dry-run mode, no changes made)");
        return Ok(());
    }

    println!();
    print!("Delete these worktrees? [y/N] ");
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    if !input.trim().eq_ignore_ascii_case("y") {
        println!("Cancelled.");
        return Ok(());
    }

    println!();

    for wt in &to_delete {
        // Remove worktree (force if submodules exist)
        let has_submodules = std::path::Path::new(&wt.path).join(".gitmodules").exists();

        let worktree_remove_args = if has_submodules {
            vec!["worktree", "remove", "--force", &wt.path]
        } else {
            vec!["worktree", "remove", &wt.path]
        };

        let status = Command::new("git")
            .args(&worktree_remove_args)
            .status()
            .map_err(|e| WmError::CommandFailed(e.to_string()))?;

        if status.success() {
            println!("Deleted: {} ({})", wt.path, wt.reason);
        } else {
            eprintln!("Failed to delete: {}", wt.path);
            continue;
        }

        // Delete the branch if it exists
        if !wt.branch.is_empty() && local_branch_exists(&wt.branch) {
            let status = Command::new("git")
                .args(["branch", "-D", &wt.branch])
                .status()
                .map_err(|e| WmError::CommandFailed(e.to_string()))?;

            if status.success() {
                println!("  Branch deleted: {}", wt.branch);
            }
        }
    }

    println!();
    println!("Done. Deleted {} worktree(s).", to_delete.len());

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
                // Skip main worktree (bare repository root)
                if path == repo_root {
                    continue;
                }

                // Check merge status
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
            }
        }
    }

    // Handle the last entry if there's no trailing newline
    if let (Some(path), Some(branch)) = (current_path, current_branch) {
        if path != repo_root {
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
        }
    }

    Ok((to_delete, to_skip))
}
