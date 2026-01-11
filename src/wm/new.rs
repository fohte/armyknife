use clap::Args;
use std::path::Path;
use std::process::Command;

use super::common::{
    BRANCH_PREFIX, Result, WmError, branch_exists, branch_to_worktree_name, get_main_branch,
    get_repo_root, local_branch_exists, remote_branch_exists,
};

/// Mode for creating a worktree
enum WorktreeAddMode<'a> {
    /// Checkout an existing local branch
    LocalBranch { branch: &'a str },
    /// Create a tracking branch from remote
    TrackRemote { branch: &'a str },
    /// Create a new branch from base
    NewBranch { branch: &'a str, base: &'a str },
    /// Force create/reset a branch from base
    ForceNewBranch { branch: &'a str, base: &'a str },
}

/// Run `git worktree add` with the specified mode
fn git_worktree_add(worktree_dir: &str, mode: WorktreeAddMode) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.args(["worktree", "add", worktree_dir]);

    match mode {
        WorktreeAddMode::LocalBranch { branch } => {
            cmd.arg(branch);
        }
        WorktreeAddMode::TrackRemote { branch } => {
            cmd.args(["-b", branch, &format!("origin/{branch}")]);
        }
        WorktreeAddMode::NewBranch { branch, base } => {
            cmd.args(["-b", branch, base]);
        }
        WorktreeAddMode::ForceNewBranch { branch, base } => {
            cmd.args(["-B", branch, base]);
        }
    }

    let status = cmd
        .status()
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    if !status.success() {
        return Err(WmError::CommandFailed("git worktree add failed".into()));
    }

    Ok(())
}

/// Add a worktree for an existing branch (local or remote)
fn add_worktree_for_branch(worktree_dir: &str, branch: &str) -> Result<()> {
    if local_branch_exists(branch) {
        git_worktree_add(worktree_dir, WorktreeAddMode::LocalBranch { branch })
    } else if remote_branch_exists(branch) {
        git_worktree_add(worktree_dir, WorktreeAddMode::TrackRemote { branch })
    } else {
        // Fallback: use as-is (should not normally happen)
        git_worktree_add(worktree_dir, WorktreeAddMode::LocalBranch { branch })
    }
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NewArgs {
    /// Branch name (existing branch will be checked out,
    /// non-existing branch will be created with fohte/ prefix)
    pub name: String,

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
    run_inner(args)?;
    Ok(())
}

fn run_inner(args: &NewArgs) -> Result<()> {
    let name = &args.name;
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

    // Remove BRANCH_PREFIX to avoid double prefix
    let name_no_prefix = name.strip_prefix(BRANCH_PREFIX).unwrap_or(name);

    // Determine action based on branch existence and flags
    if args.force {
        // Force create new branch with BRANCH_PREFIX
        let main_branch = get_main_branch()?;
        let base_branch = args
            .from
            .clone()
            .unwrap_or_else(|| format!("origin/{main_branch}"));
        let branch = format!("{BRANCH_PREFIX}{name_no_prefix}");

        git_worktree_add(
            &worktree_dir,
            WorktreeAddMode::ForceNewBranch {
                branch: &branch,
                base: &base_branch,
            },
        )?;
    } else if branch_exists(name) {
        // Branch exists with the exact name provided
        add_worktree_for_branch(&worktree_dir, name)?;
    } else {
        let branch_with_prefix = format!("{BRANCH_PREFIX}{name_no_prefix}");
        if branch_exists(&branch_with_prefix) {
            // Branch exists with BRANCH_PREFIX
            add_worktree_for_branch(&worktree_dir, &branch_with_prefix)?;
        } else {
            // Branch doesn't exist, create new one with BRANCH_PREFIX
            let main_branch = get_main_branch()?;
            let base_branch = args
                .from
                .clone()
                .unwrap_or_else(|| format!("origin/{main_branch}"));
            let branch = format!("{BRANCH_PREFIX}{name_no_prefix}");

            git_worktree_add(
                &worktree_dir,
                WorktreeAddMode::NewBranch {
                    branch: &branch,
                    base: &base_branch,
                },
            )?;
        }
    }

    // Build claude command with optional prompt
    // Use a temp file + keep() so the file survives until the shell reads it.
    // We use `claude "$(cat '<path>')" ; rm '<path>'` which is POSIX-compliant,
    // preserves exact content, and cleans up the temp file after use.
    let claude_cmd = if let Some(prompt) = &args.prompt {
        let prompt_file = tempfile::Builder::new()
            .prefix("claude-prompt-")
            .suffix(".txt")
            .tempfile()
            .map_err(|e| WmError::CommandFailed(format!("Failed to create temp file: {e}")))?;

        std::fs::write(prompt_file.path(), prompt)
            .map_err(|e| WmError::CommandFailed(format!("Failed to write prompt: {e}")))?;

        // keep() persists the file on disk (it won't be deleted on drop)
        let prompt_path = prompt_file
            .into_temp_path()
            .keep()
            .map_err(|e| WmError::CommandFailed(format!("Failed to persist temp file: {e}")))?;

        // Use shlex for safe shell escaping (handles spaces, quotes, metacharacters)
        let path_str = prompt_path.display().to_string();
        let escaped_path = shlex::try_quote(&path_str)
            .map_err(|_| WmError::CommandFailed("Failed to escape path".into()))?;

        // Read prompt, pass to claude, then delete the temp file
        format!("claude \"$(cat {escaped_path})\" ; rm {escaped_path}")
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
    if let Some(output) = Command::new("tmux-name")
        .args(["session", repo_root])
        .output()
        .ok()
        .filter(|o| o.status.success())
    {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }

    // Fallback: use the directory name
    Path::new(repo_root)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default")
        .to_string()
}
