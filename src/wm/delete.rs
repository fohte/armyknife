use clap::Args;
use git2::{BranchType, Repository, WorktreePruneOptions};
use std::io::{self, Write};
use std::process::Command;

use super::error::{Result, WmError};
use super::git::{branch_to_worktree_name, get_merge_status, get_repo_root, local_branch_exists};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct DeleteArgs {
    /// Worktree path or name (default: current directory)
    pub worktree: Option<String>,

    /// Force delete without confirmation even if not merged
    #[arg(short, long)]
    pub force: bool,
}

pub fn run(args: &DeleteArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    run_inner(args)?;
    Ok(())
}

fn run_inner(args: &DeleteArgs) -> Result<()> {
    let worktree_path = resolve_worktree_path(args.worktree.as_deref())?;

    let repo = Repository::open_from_env().map_err(|_| WmError::NotInGitRepo)?;

    // Get the main repo (if we're in a worktree, get the parent)
    let main_repo = if repo.is_worktree() {
        let commondir = repo.commondir();
        Repository::open(commondir.parent().ok_or(WmError::NotInGitRepo)?)
            .map_err(|_| WmError::NotInGitRepo)?
    } else {
        repo
    };

    // Verify this is actually a worktree by checking against the worktree list
    let worktree_name = find_worktree_name(&main_repo, &worktree_path)?;

    // Get the branch name associated with this worktree
    let branch_name = get_worktree_branch(&main_repo, &worktree_name)?;

    // Check if we're in a tmux session and current pane is in the worktree
    let target_window_id = get_current_tmux_window_if_in_worktree(&worktree_path);

    // Check if the branch can be safely deleted before deleting worktree
    if let Some(ref branch) = branch_name.as_ref().filter(|b| local_branch_exists(b)) {
        let merge_status = get_merge_status(branch);
        if !merge_status.is_merged() && !args.force {
            eprintln!(
                "Warning: Branch '{}' is not merged ({})",
                branch,
                merge_status.reason()
            );
            print!("Delete anyway? [y/N] ");
            io::stdout().flush().ok();

            let mut input = String::new();
            io::stdin().read_line(&mut input).ok();
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("Cancelled.");
                return Err(WmError::Cancelled);
            }
        }
    }

    // Remove the worktree using git2
    let worktree = main_repo
        .find_worktree(&worktree_name)
        .map_err(|e| WmError::CommandFailed(format!("Failed to find worktree: {e}")))?;

    let mut prune_opts = WorktreePruneOptions::new();
    prune_opts.valid(true).working_tree(true);

    worktree
        .prune(Some(&mut prune_opts))
        .map_err(|e| WmError::CommandFailed(format!("Failed to remove worktree: {e}")))?;

    println!("Worktree removed: {worktree_path}");

    // Delete the branch if it exists
    if let Some(branch) = branch_name.filter(|b| local_branch_exists(b)) {
        if let Ok(mut branch_ref) = main_repo.find_branch(&branch, BranchType::Local) {
            if branch_ref.delete().is_ok() {
                println!("Branch deleted: {branch}");
            }
        }
    }

    // Close the original tmux window (identified by window ID)
    if let Some(window_id) = target_window_id {
        Command::new("tmux")
            .args(["kill-window", "-t", &window_id])
            .status()
            .ok();
    }

    Ok(())
}

/// Resolve the worktree path from the argument or current directory
fn resolve_worktree_path(worktree_arg: Option<&str>) -> Result<String> {
    if let Some(arg) = worktree_arg {
        // First, try to treat the argument as an existing path
        if let Ok(path) = std::fs::canonicalize(arg) {
            return Ok(path.to_string_lossy().to_string());
        }

        // Fall back to resolving the value as a branch/worktree name
        let repo_root = get_repo_root()?;
        let worktree_name = branch_to_worktree_name(arg);
        let candidate_path = format!("{repo_root}/.worktrees/{worktree_name}");

        if std::path::Path::new(&candidate_path).exists() {
            let path = std::fs::canonicalize(&candidate_path)
                .map_err(|e| WmError::CommandFailed(e.to_string()))?;
            return Ok(path.to_string_lossy().to_string());
        }

        Err(WmError::WorktreeNotFound(arg.to_string()))
    } else {
        // Use current directory
        std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .map_err(|e| WmError::CommandFailed(e.to_string()))
    }
}

/// Find the worktree name from its path
fn find_worktree_name(repo: &Repository, worktree_path: &str) -> Result<String> {
    let worktrees = repo
        .worktrees()
        .map_err(|e| WmError::CommandFailed(e.message().to_string()))?;

    for name in worktrees.iter().flatten() {
        if let Ok(wt) = repo.find_worktree(name) {
            let wt_path = wt.path().to_string_lossy();
            // Compare paths (handle trailing slash differences)
            let wt_path_normalized = wt_path.trim_end_matches('/');
            let worktree_path_normalized = worktree_path.trim_end_matches('/');
            if wt_path_normalized == worktree_path_normalized {
                return Ok(name.to_string());
            }
        }
    }

    Err(WmError::WorktreeNotFound(worktree_path.to_string()))
}

/// Get the branch name associated with a worktree
fn get_worktree_branch(repo: &Repository, worktree_name: &str) -> Result<Option<String>> {
    let worktree = match repo.find_worktree(worktree_name) {
        Ok(wt) => wt,
        Err(_) => return Ok(None),
    };

    // Open the worktree repository to get its HEAD
    let wt_repo = match Repository::open_from_worktree(&worktree) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };

    let head = match wt_repo.head() {
        Ok(h) => h,
        Err(_) => return Ok(None),
    };

    Ok(head.shorthand().map(|s| s.to_string()))
}

/// Get the current tmux window ID if we're in a tmux session and in the worktree
fn get_current_tmux_window_if_in_worktree(worktree_path: &str) -> Option<String> {
    if std::env::var("TMUX").is_err() {
        return None;
    }

    let pane_path = Command::new("tmux")
        .args(["display-message", "-p", "#{pane_current_path}"])
        .output()
        .ok()?;

    if !pane_path.status.success() {
        return None;
    }

    let current_path = String::from_utf8_lossy(&pane_path.stdout)
        .trim()
        .to_string();

    // Use Path::starts_with for proper path comparison (avoids /tmp/foo matching /tmp/foo2)
    if std::path::Path::new(&current_path).starts_with(worktree_path) {
        let window_id = Command::new("tmux")
            .args(["display-message", "-p", "#{window_id}"])
            .output()
            .ok()?;

        if window_id.status.success() {
            return Some(
                String::from_utf8_lossy(&window_id.stdout)
                    .trim()
                    .to_string(),
            );
        }
    }

    None
}
