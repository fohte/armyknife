use anyhow::{Context, bail};
use clap::Args;
use git2::Repository;
use std::io::{self, Write};

use super::error::{Result, WmError};
use super::git::{branch_to_worktree_name, get_merge_status, get_repo_root, local_branch_exists};
use super::worktree::{
    delete_branch_if_exists, delete_worktree, find_worktree_name, get_main_repo,
    get_worktree_branch,
};
use crate::shared::tmux;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct DeleteArgs {
    /// Worktree path or name (default: current directory)
    pub worktree: Option<String>,

    /// Force delete without confirmation even if not merged
    #[arg(short, long)]
    pub force: bool,
}

pub async fn run(args: &DeleteArgs) -> Result<()> {
    let worktree_path = resolve_worktree_path(args.worktree.as_deref())?;

    let repo = Repository::open_from_env().map_err(|_| WmError::NotInGitRepo)?;
    let main_repo = get_main_repo(&repo)?;

    let worktree_name = find_worktree_name(&main_repo, &worktree_path)?;
    let branch_name = get_worktree_branch(&main_repo, &worktree_name);

    // Check if we're in a tmux session and current pane is in the worktree
    let target_window_id = tmux::get_window_id_if_in_path(&worktree_path);

    // Check if the branch can be safely deleted before deleting worktree
    if let Some(ref branch) = branch_name.as_ref().filter(|b| local_branch_exists(b)) {
        let merge_status = get_merge_status(branch).await;
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
                return Err(WmError::Cancelled.into());
            }
        }
    }

    // Remove the worktree
    if !delete_worktree(&main_repo, &worktree_name)? {
        bail!("Failed to remove worktree: {worktree_path}");
    }
    println!("Worktree removed: {worktree_path}");

    // Delete the branch if it exists
    if let Some(branch) = branch_name
        && delete_branch_if_exists(&main_repo, &branch)
    {
        println!("Branch deleted: {branch}");
    }

    // Close the original tmux window (identified by window ID)
    if let Some(window_id) = target_window_id {
        tmux::kill_window(&window_id);
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
                .context("Failed to canonicalize worktree path")?;
            return Ok(path.to_string_lossy().to_string());
        }

        Err(WmError::WorktreeNotFound(arg.to_string()).into())
    } else {
        // Use current directory
        Ok(std::env::current_dir()
            .context("Failed to get current directory")?
            .to_string_lossy()
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::TestRepo;

    #[test]
    fn resolve_worktree_path_with_existing_path() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature");

        let wt_path = test_repo.worktree_path("feature");
        let result = resolve_worktree_path(Some(wt_path.to_str().unwrap())).unwrap();

        assert_eq!(result, wt_path.to_string_lossy().to_string());
    }

    #[test]
    fn resolve_worktree_path_with_nonexistent_returns_error() {
        let result = resolve_worktree_path(Some("/nonexistent/path/to/worktree"));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_worktree_path_with_none_returns_current_dir() {
        let current = std::env::current_dir().unwrap();
        let result = resolve_worktree_path(None).unwrap();

        assert_eq!(result, current.to_string_lossy().to_string());
    }
}
