use anyhow::{Context, Result};

use crate::infra::tmux;
use crate::shared::config::Config;

/// Setup a tmux window using the configured layout.
pub(super) fn setup_tmux_window(
    repo_root: &str,
    worktree_dir: &str,
    worktree_name: &str,
    prompt: Option<&str>,
    config: &Config,
    env_vars: &[(&str, &str)],
    background: bool,
) -> Result<()> {
    let target_session = tmux::get_session_name(repo_root, &config.wm.worktrees_dir);

    tmux::ensure_session(&target_session, repo_root).context("Failed to ensure tmux session")?;

    tmux::layout::build_layout(
        &target_session,
        worktree_dir,
        worktree_name,
        &config.wm.layout,
        prompt,
        env_vars,
        background,
    )
    .context("Failed to create tmux layout")?;

    if !background {
        tmux::switch_to_session(&target_session).context("Failed to switch to tmux session")?;
    }

    Ok(())
}
