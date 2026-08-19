use anyhow::Result;
use clap::Args;

use crate::commands::cc::new;
use crate::commands::cc::new::CommonNewArgs;

/// [DEPRECATED] Create a new Git worktree for a branch.
/// Use `a cc new --worktree` instead.
#[derive(Args, Clone, PartialEq, Eq)]
pub struct WmNewArgs {
    /// Branch name (existing branch will be checked out,
    /// non-existing branch will be created with fohte/ prefix).
    /// Optional when --prompt is provided (auto-generated from prompt).
    pub name: Option<String>,

    /// Base branch for new branch creation (default: origin/main or origin/master)
    #[arg(long)]
    pub from: Option<String>,

    /// Force create new branch even if it already exists
    #[arg(long)]
    pub force: bool,

    #[command(flatten)]
    pub common: CommonNewArgs,

    /// Skip the post-worktree-create hook.
    /// Useful when the hook itself is broken and needs to be fixed inside the new worktree.
    #[arg(long)]
    pub skip_hooks: bool,
}

pub fn run(args: &WmNewArgs) -> Result<()> {
    eprintln!("Warning: 'wm new' is deprecated. Use 'cc new --worktree' instead.");
    new::run(&new::NewArgs {
        worktree: Some(args.name.clone()),
        from: args.from.clone(),
        force: args.force,
        common: args.common.clone(),
        skip_hooks: args.skip_hooks,
    })
}
