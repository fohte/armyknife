use clap::Args;
use std::path::Path;
use std::process::Command;

use super::common::{
    Result, WmError, branch_exists, branch_to_worktree_name, get_main_branch, get_repo_root,
    local_branch_exists, remote_branch_exists,
};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NewArgs {
    /// Branch name (existing branch will be checked out,
    /// non-existing branch will be created with fohte/ prefix)
    pub name: Option<String>,

    /// Base branch for new branch creation (default: origin/main or origin/master)
    #[arg(long)]
    pub from: Option<String>,

    /// Force create new branch even if it already exists
    #[arg(long)]
    pub force: bool,

    /// Initial prompt to send to Claude Code
    #[arg(long)]
    pub prompt: Option<String>,
}

pub fn run(args: &NewArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let name = args
        .name
        .clone()
        .ok_or_else(|| WmError::CommandFailed("Branch name is required".into()))?;

    run_inner(args, &name)?;
    Ok(())
}

fn run_inner(args: &NewArgs, name: &str) -> Result<()> {
    let repo_root = get_repo_root()?;

    // Determine worktree directory name from branch name
    let worktree_name = branch_to_worktree_name(name);
    let worktrees_dir = format!("{repo_root}/.worktrees");
    let worktree_dir = format!("{worktrees_dir}/{worktree_name}");

    // Ensure .worktrees directory exists
    std::fs::create_dir_all(&worktrees_dir).map_err(|e| {
        WmError::CommandFailed(format!("Failed to create .worktrees directory: {e}"))
    })?;

    // Fetch with prune
    let fetch_status = Command::new("git")
        .args(["fetch", "-p"])
        .status()
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    if !fetch_status.success() {
        return Err(WmError::CommandFailed("git fetch failed".into()));
    }

    // Remove fohte/ prefix to avoid fohte/fohte/<name>
    let name_no_prefix = name.strip_prefix("fohte/").unwrap_or(name);

    // Determine action based on branch existence and flags
    if args.force {
        // Force create new branch with fohte/ prefix
        let main_branch = get_main_branch()?;
        let base_branch = args
            .from
            .clone()
            .unwrap_or_else(|| format!("origin/{main_branch}"));
        let branch = format!("fohte/{name_no_prefix}");

        let status = Command::new("git")
            .args([
                "worktree",
                "add",
                &worktree_dir,
                "-B",
                &branch,
                &base_branch,
            ])
            .status()
            .map_err(|e| WmError::CommandFailed(e.to_string()))?;

        if !status.success() {
            return Err(WmError::CommandFailed("git worktree add failed".into()));
        }
    } else if branch_exists(name) {
        // Branch exists with the exact name provided
        if local_branch_exists(name) {
            // If local branch exists, use it as-is
            let status = Command::new("git")
                .args(["worktree", "add", &worktree_dir, name])
                .status()
                .map_err(|e| WmError::CommandFailed(e.to_string()))?;

            if !status.success() {
                return Err(WmError::CommandFailed("git worktree add failed".into()));
            }
        } else if remote_branch_exists(name) {
            // If remote branch exists, create local tracking branch
            let status = Command::new("git")
                .args([
                    "worktree",
                    "add",
                    &worktree_dir,
                    "-b",
                    name,
                    &format!("origin/{name}"),
                ])
                .status()
                .map_err(|e| WmError::CommandFailed(e.to_string()))?;

            if !status.success() {
                return Err(WmError::CommandFailed("git worktree add failed".into()));
            }
        } else {
            // Fallback: use as-is
            let status = Command::new("git")
                .args(["worktree", "add", &worktree_dir, name])
                .status()
                .map_err(|e| WmError::CommandFailed(e.to_string()))?;

            if !status.success() {
                return Err(WmError::CommandFailed("git worktree add failed".into()));
            }
        }
    } else {
        let branch_with_prefix = format!("fohte/{name_no_prefix}");
        if branch_exists(&branch_with_prefix) {
            // Branch exists with fohte/ prefix
            if local_branch_exists(&branch_with_prefix) {
                let status = Command::new("git")
                    .args(["worktree", "add", &worktree_dir, &branch_with_prefix])
                    .status()
                    .map_err(|e| WmError::CommandFailed(e.to_string()))?;

                if !status.success() {
                    return Err(WmError::CommandFailed("git worktree add failed".into()));
                }
            } else if remote_branch_exists(&branch_with_prefix) {
                let status = Command::new("git")
                    .args([
                        "worktree",
                        "add",
                        &worktree_dir,
                        "-b",
                        &branch_with_prefix,
                        &format!("origin/{branch_with_prefix}"),
                    ])
                    .status()
                    .map_err(|e| WmError::CommandFailed(e.to_string()))?;

                if !status.success() {
                    return Err(WmError::CommandFailed("git worktree add failed".into()));
                }
            } else {
                let status = Command::new("git")
                    .args(["worktree", "add", &worktree_dir, &branch_with_prefix])
                    .status()
                    .map_err(|e| WmError::CommandFailed(e.to_string()))?;

                if !status.success() {
                    return Err(WmError::CommandFailed("git worktree add failed".into()));
                }
            }
        } else {
            // Branch doesn't exist, create new one with fohte/ prefix
            let main_branch = get_main_branch()?;
            let base_branch = args
                .from
                .clone()
                .unwrap_or_else(|| format!("origin/{main_branch}"));
            let branch = format!("fohte/{name_no_prefix}");

            let status = Command::new("git")
                .args([
                    "worktree",
                    "add",
                    &worktree_dir,
                    "-b",
                    &branch,
                    &base_branch,
                ])
                .status()
                .map_err(|e| WmError::CommandFailed(e.to_string()))?;

            if !status.success() {
                return Err(WmError::CommandFailed("git worktree add failed".into()));
            }
        }
    }

    // Build claude command with optional prompt
    // Note: We pass the prompt as a quoted argument directly to avoid temp file issues
    let claude_cmd = if let Some(prompt) = &args.prompt {
        // Escape single quotes in prompt for shell
        let escaped = prompt.replace('\'', "'\\''");
        format!("claude $'{escaped}'")
    } else {
        "claude".to_string()
    };

    // Determine target tmux session from repository root
    let target_session = get_tmux_session_name(&repo_root);

    // Create session if it doesn't exist
    let session_exists = Command::new("tmux")
        .args(["has-session", "-t", &target_session])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !session_exists {
        let status = Command::new("tmux")
            .args(["new-session", "-ds", &target_session, "-c", &repo_root])
            .status()
            .map_err(|e| WmError::CommandFailed(e.to_string()))?;

        if !status.success() {
            return Err(WmError::CommandFailed("tmux new-session failed".into()));
        }
    }

    // Create a tmux window in the target session with split layout
    // left pane: neovim, right pane: claude code
    let status = Command::new("tmux")
        .args([
            "new-window",
            "-t",
            &target_session,
            "-c",
            &worktree_dir,
            "-n",
            &worktree_name,
            ";",
            "split-window",
            "-h",
            "-c",
            &worktree_dir,
            ";",
            "select-pane",
            "-t",
            "1",
            ";",
            "send-keys",
            "nvim",
            "C-m",
            ";",
            "select-pane",
            "-t",
            "2",
            ";",
            "send-keys",
            &claude_cmd,
            "C-m",
            ";",
            "select-pane",
            "-t",
            "1",
        ])
        .status()
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    if !status.success() {
        return Err(WmError::CommandFailed("tmux new-window failed".into()));
    }

    // Switch to the target session if in tmux and not already in it
    if std::env::var("TMUX").is_ok() {
        let current_session = Command::new("tmux")
            .args(["display-message", "-p", "#{session_name}"])
            .output()
            .map_err(|e| WmError::CommandFailed(e.to_string()))?;

        if current_session.status.success() {
            let current = String::from_utf8_lossy(&current_session.stdout)
                .trim()
                .to_string();
            if current != target_session {
                Command::new("tmux")
                    .args(["switch-client", "-t", &target_session])
                    .status()
                    .map_err(|e| WmError::CommandFailed(e.to_string()))?;
            }
        }
    }

    Ok(())
}

/// Get tmux session name from repository root (equivalent to tmux-name session)
fn get_tmux_session_name(repo_root: &str) -> String {
    // Try tmux-name command first
    if let Ok(output) = Command::new("tmux-name")
        .args(["session", repo_root])
        .output()
    {
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }

    // Fallback: use the directory name
    Path::new(repo_root)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default")
        .to_string()
}
