use clap::Args;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct DeleteArgs {
    /// Worktree path or name (default: current directory)
    pub worktree: Option<String>,

    /// Force delete without confirmation even if the branch is neither merged nor closed
    #[arg(short, long)]
    pub force: bool,

    /// Skip the pre-worktree-delete hook
    #[arg(long)]
    pub skip_hooks: bool,
}

pub async fn run(args: &DeleteArgs) -> anyhow::Result<()> {
    crate::shared::worktree_delete::run(args.worktree.as_deref(), args.force, args.skip_hooks).await
}
