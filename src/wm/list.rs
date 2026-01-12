use clap::Args;
use git2::Repository;

use super::error::WmError;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ListArgs {}

pub fn run(_args: &ListArgs) -> Result<(), Box<dyn std::error::Error>> {
    let repo = Repository::open_from_env().map_err(|_| WmError::NotInGitRepo)?;

    // Get the main worktree path
    let main_path = if repo.is_worktree() {
        repo.commondir()
            .parent()
            .ok_or(WmError::NotInGitRepo)?
            .to_path_buf()
    } else {
        repo.workdir().ok_or(WmError::NotInGitRepo)?.to_path_buf()
    };

    // Print main worktree
    let head = repo.head().ok();
    let main_branch = head
        .as_ref()
        .and_then(|h| h.shorthand())
        .unwrap_or("(unknown)");
    let main_commit = head
        .as_ref()
        .and_then(|h| h.peel_to_commit().ok())
        .map(|c| c.id().to_string())
        .map(|s| s[..7].to_string())
        .unwrap_or_else(|| "(none)".to_string());
    println!(
        "{:<50} {} [{}]",
        main_path.display(),
        main_commit,
        main_branch
    );

    // List linked worktrees
    let worktrees = repo
        .worktrees()
        .map_err(|e| WmError::CommandFailed(e.message().to_string()))?;
    for name in worktrees.iter().flatten() {
        if let Ok(wt) = repo.find_worktree(name) {
            let wt_path = wt.path();
            // Open the worktree repository to get its HEAD
            if let Ok(wt_repo) = Repository::open(wt_path) {
                let wt_head = wt_repo.head().ok();
                let branch = wt_head
                    .as_ref()
                    .and_then(|h| h.shorthand())
                    .unwrap_or("(unknown)");
                let commit = wt_head
                    .as_ref()
                    .and_then(|h| h.peel_to_commit().ok())
                    .map(|c| c.id().to_string())
                    .map(|s| s[..7].to_string())
                    .unwrap_or_else(|| "(none)".to_string());
                println!("{:<50} {} [{}]", wt_path.display(), commit, branch);
            }
        }
    }

    Ok(())
}
