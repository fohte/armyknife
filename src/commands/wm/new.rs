use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

use crate::commands::cc::new;

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

    /// Initial prompt to send to Claude Code.
    /// When provided without a branch name, the branch name is auto-generated from this prompt.
    #[arg(long)]
    pub prompt: Option<String>,

    /// Mark this invocation as coming from another Claude Code session.
    /// Wraps the prompt with delegation context (branch, base, directories).
    #[arg(long)]
    pub agent: bool,

    /// Label for the new session (displayed in cc watch).
    /// When not specified, the session will get its label via the
    /// user-prompt-submit hook (auto-generation from prompt).
    #[arg(long)]
    pub label: Option<String>,

    /// Parent session ID for tree view hierarchy.
    /// Sets ARMYKNIFE_ANCESTOR_SESSION_IDS for the child session.
    #[arg(long)]
    pub parent_session_id: Option<String>,

    /// Path to the target repository.
    /// When specified, operates on the given repository instead of the current directory.
    #[arg(short = 'R', long)]
    pub repo: Option<PathBuf>,

    /// Skip the post-worktree-create hook.
    /// Useful when the hook itself is broken and needs to be fixed inside the new worktree.
    #[arg(long)]
    pub skip_hooks: bool,
}

pub fn run(args: &WmNewArgs) -> Result<()> {
    new::run(&new::NewArgs {
        worktree: args.name.clone(),
        from: args.from.clone(),
        force: args.force,
        prompt: args.prompt.clone(),
        agent: args.agent,
        label: args.label.clone(),
        parent_session_id: args.parent_session_id.clone(),
        repo: args.repo.clone(),
        skip_hooks: args.skip_hooks,
    })
}
