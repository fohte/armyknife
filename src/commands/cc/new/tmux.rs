use anyhow::{Context, Result};

use crate::infra::tmux;
use crate::shared::config::{Config, LayoutNode};

/// Inputs for setting up a tmux window, grouped to keep `setup_tmux_window`'s
/// argument count in check.
pub(super) struct TmuxWindowSpec<'a> {
    pub repo_root: &'a str,
    pub cwd: &'a str,
    pub window_name: &'a str,
    pub layout: &'a LayoutNode,
    pub model: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub env_vars: &'a [(&'a str, &'a str)],
    pub background: bool,
    /// Turns `automatic-rename` back on right after window creation, undoing
    /// tmux's default of disabling it whenever a window is created with an
    /// explicit name (`-n`). Set for the no-worktree window, whose PID-based
    /// name is a meaningless placeholder; left off for the worktree window,
    /// whose name is the branch/worktree name and should stay displayed.
    pub restore_automatic_rename: bool,
}

/// Setup a tmux window with the given layout.
pub(super) fn setup_tmux_window(spec: TmuxWindowSpec, config: &Config) -> Result<()> {
    let target_session = tmux::get_session_name(spec.repo_root, &config.wm.worktrees_dir);

    tmux::ensure_session(&target_session, spec.repo_root)
        .context("Failed to ensure tmux session")?;

    tmux::layout::build_layout(tmux::layout::LayoutSpec {
        session: &target_session,
        cwd: spec.cwd,
        window_name: spec.window_name,
        layout: spec.layout,
        model: spec.model,
        prompt: spec.prompt,
        env_vars: spec.env_vars,
        background: spec.background,
        restore_automatic_rename: spec.restore_automatic_rename,
    })
    .context("Failed to create tmux layout")?;

    if !spec.background {
        tmux::switch_to_session(&target_session).context("Failed to switch to tmux session")?;
    }

    Ok(())
}
