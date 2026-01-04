use clap::Args;
use std::io::{self, Write};
use std::process::Command;

use super::common::{
    Result, WmError, branch_to_worktree_name, get_merge_status, get_repo_root, local_branch_exists,
};

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

    // Verify this is actually a worktree
    let worktree_list = Command::new("git")
        .args(["worktree", "list"])
        .output()
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    if !worktree_list.status.success() {
        return Err(WmError::CommandFailed("git worktree list failed".into()));
    }

    let list_output = String::from_utf8_lossy(&worktree_list.stdout);
    if !list_output.contains(&worktree_path) {
        return Err(WmError::WorktreeNotFound(worktree_path));
    }

    // Get the branch name associated with this worktree
    let branch_name = get_worktree_branch(&worktree_path)?;

    // Check if we're in a tmux session and current pane is in the worktree
    let target_window_id = get_current_tmux_window_if_in_worktree(&worktree_path);

    // Check if the branch can be safely deleted before deleting worktree
    if let Some(ref branch) = branch_name {
        if local_branch_exists(branch) {
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
    }

    // Remove the worktree (force if submodules exist)
    let has_submodules = std::path::Path::new(&worktree_path)
        .join(".gitmodules")
        .exists();

    let worktree_remove_args = if has_submodules {
        vec!["worktree", "remove", "--force", &worktree_path]
    } else {
        vec!["worktree", "remove", &worktree_path]
    };

    let status = Command::new("git")
        .args(&worktree_remove_args)
        .status()
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    if !status.success() {
        return Err(WmError::CommandFailed("git worktree remove failed".into()));
    }

    println!("Worktree removed: {worktree_path}");

    // Delete the branch if it exists
    if let Some(branch) = branch_name {
        if local_branch_exists(&branch) {
            let status = Command::new("git")
                .args(["branch", "-D", &branch])
                .status()
                .map_err(|e| WmError::CommandFailed(e.to_string()))?;

            if status.success() {
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

/// Get the branch name associated with a worktree
fn get_worktree_branch(worktree_path: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut current_worktree: Option<&str> = None;

    for line in stdout.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current_worktree = Some(path);
        } else if let Some(branch_ref) = line.strip_prefix("branch ") {
            if current_worktree == Some(worktree_path) {
                // Remove refs/heads/ prefix
                let branch = branch_ref.strip_prefix("refs/heads/").unwrap_or(branch_ref);
                return Ok(Some(branch.to_string()));
            }
        }
    }

    Ok(None)
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
