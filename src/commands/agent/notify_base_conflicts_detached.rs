//! Internal detached worker for worktree base-conflict notifications.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use crate::shared::base_conflict_notify;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NotifyBaseConflictsDetachedArgs {
    /// Repository whose worktree branches should be checked.
    #[arg(long)]
    pub repo: PathBuf,

    /// Worktree paths being removed by the caller.
    #[arg(long = "exclude-path", value_name = "PATH")]
    pub exclude_paths: Vec<PathBuf>,
}

pub fn run(args: &NotifyBaseConflictsDetachedArgs) -> Result<()> {
    base_conflict_notify::notify_conflicting_worktrees(&args.repo, &args.exclude_paths);
    Ok(())
}
