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
}

/// Setup a tmux window with the given layout.
pub(super) fn setup_tmux_window(spec: TmuxWindowSpec, config: &Config) -> Result<()> {
    let target_session = tmux::get_session_name(spec.repo_root, &config.wm.worktrees_dir);

    tmux::ensure_session(&target_session, spec.repo_root)
        .context("Failed to ensure tmux session")?;

    tmux::layout::build_layout(
        &target_session,
        spec.cwd,
        spec.window_name,
        spec.layout,
        spec.model,
        spec.prompt,
        spec.env_vars,
        spec.background,
    )
    .context("Failed to create tmux layout")?;

    if !spec.background {
        tmux::switch_to_session(&target_session).context("Failed to switch to tmux session")?;
    }

    Ok(())
}
