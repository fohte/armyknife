use anyhow::{Context, bail};
use clap::Args;
use git2::Repository;
use std::io::{self, Write};

use super::error::{Result, WmError};
use super::git::{branch_to_worktree_name, get_merge_status, get_repo_root, local_branch_exists};
use super::worktree::{find_worktree_name, get_main_repo, get_worktree_branch};
use crate::infra::tmux;
use crate::shared::cleanup;
use crate::shared::config::load_config;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct DeleteArgs {
    /// Worktree path or name (default: current directory)
    pub worktree: Option<String>,

    /// Force delete without confirmation even if not merged
    #[arg(short, long)]
    pub force: bool,
}

pub async fn run(args: &DeleteArgs) -> Result<()> {
    let config = load_config()?;
    let worktree_path = resolve_worktree_path(
        args.worktree.as_deref(),
        &config.wm.worktrees_dir,
        &config.wm.branch_prefix,
    )?;

    let repo = Repository::open_from_env().map_err(|_| WmError::NotInGitRepo)?;
    let main_repo = get_main_repo(&repo)?;

    let worktree_name = find_worktree_name(&main_repo, &worktree_path)?;

    // Check merge status before deletion (needs worktree to still exist)
    check_merge_status(&main_repo, &worktree_name, args.force).await?;

    // Capture the current tmux window ID before cleanup deletes it,
    // so we can close the window we're sitting in
    let current_window_id = tmux::get_window_id_if_in_path(&worktree_path);

    let worktree_abs = std::path::Path::new(&worktree_path);
    let result = cleanup::cleanup_worktree_by_name(&main_repo, &worktree_name, worktree_abs)?;

    if !result.worktree_deleted {
        bail!("Failed to remove worktree: {worktree_path}");
    }
    println!("Worktree removed: {worktree_path}");

    if let Some(branch) = &result.branch_deleted {
        println!("Branch deleted: {branch}");
    }
    if result.sessions_cleaned > 0 {
        println!("Sessions cleaned: {}", result.sessions_cleaned);
    }

    // Close the current tmux window if we're inside the deleted worktree.
    // cleanup_worktree_by_name uses get_window_ids_in_path which queries all
    // panes globally, but the current window may have already been captured
    // above via get_window_id_if_in_path. Ensure it's closed.
    if let Some(window_id) = current_window_id {
        let _ = tmux::kill_window(&window_id);
    }

    Ok(())
}

/// Checks if the worktree's branch is merged and prompts for confirmation if not.
async fn check_merge_status(repo: &Repository, worktree_name: &str, force: bool) -> Result<()> {
    let branch_name = get_worktree_branch(repo, worktree_name);

    if let Some(ref branch) = branch_name.as_ref().filter(|b| local_branch_exists(b)) {
        let merge_status = get_merge_status(branch).await;
        if !merge_status.is_merged() && !force {
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

    Ok(())
}

/// Resolve the worktree path from the argument or current directory
fn resolve_worktree_path(
    worktree_arg: Option<&str>,
    worktrees_dir: &str,
    branch_prefix: &str,
) -> Result<String> {
    if let Some(arg) = worktree_arg {
        // First, try to treat the argument as an existing path
        if let Ok(path) = std::fs::canonicalize(arg) {
            return Ok(path.to_string_lossy().to_string());
        }

        // Fall back to resolving the value as a branch/worktree name
        let repo_root = get_repo_root()?;
        let worktree_name = branch_to_worktree_name(arg, branch_prefix);
        let candidate_path = format!("{repo_root}/{worktrees_dir}/{worktree_name}");

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
        let result =
            resolve_worktree_path(Some(wt_path.to_str().unwrap()), ".worktrees", "fohte/").unwrap();

        assert_eq!(result, wt_path.to_string_lossy().to_string());
    }

    #[test]
    fn resolve_worktree_path_with_nonexistent_returns_error() {
        let result = resolve_worktree_path(
            Some("/nonexistent/path/to/worktree"),
            ".worktrees",
            "fohte/",
        );
        assert!(result.is_err());
    }

    #[test]
    fn resolve_worktree_path_with_none_returns_current_dir() {
        let current = std::env::current_dir().unwrap();
        let result = resolve_worktree_path(None, ".worktrees", "fohte/").unwrap();

        assert_eq!(result, current.to_string_lossy().to_string());
    }
}
