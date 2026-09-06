use anyhow::{Context, Result};

use crate::infra::tmux;
use crate::shared::config::{Config, LayoutNode, PaneConfig};

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
        common: tmux::layout::TmuxSessionSpec {
            session: &target_session,
            cwd: spec.cwd,
            model: spec.model,
            prompt: spec.prompt,
            env_vars: spec.env_vars,
            background: spec.background,
        },
        window_name: spec.window_name,
        layout: spec.layout,
        restore_automatic_rename: spec.restore_automatic_rename,
    })
    .context("Failed to create tmux layout")?;

    if !spec.background {
        tmux::switch_to_session(&target_session).context("Failed to switch to tmux session")?;
    }

    Ok(())
}

/// Inputs for `setup_no_worktree_window`, grouped to keep its argument count
/// in check.
pub(super) struct NoWorktreeWindowSpec<'a> {
    pub repo_root: &'a str,
    pub cwd: &'a str,
    pub model: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub env_vars: &'a [(&'a str, &'a str)],
    pub background: bool,
}

/// Opens a new tmux window in the target repo's own tmux session for `a cc
/// new` without `--worktree`, used when the target repo differs from the
/// invoking Claude Code session's repo (see `should_open_window` in
/// `session_mode`) -- splitting the caller's pane would otherwise land the
/// new session in the wrong repo's window.
pub(super) fn setup_no_worktree_window(spec: NoWorktreeWindowSpec, config: &Config) -> Result<()> {
    // PID-based placeholder: there's no worktree/branch name to use here, and
    // `restore_automatic_rename` below lets tmux relabel the window once
    // `claude` starts instead of keeping this name displayed.
    let window_name = format!("claude-{}", std::process::id());
    let layout = LayoutNode::Pane(PaneConfig {
        command: "claude".to_string(),
        focus: true,
    });

    setup_tmux_window(
        TmuxWindowSpec {
            repo_root: spec.repo_root,
            cwd: spec.cwd,
            window_name: &window_name,
            layout: &layout,
            model: spec.model,
            prompt: spec.prompt,
            env_vars: spec.env_vars,
            background: spec.background,
            restore_automatic_rename: true,
        },
        config,
    )
}

/// Inputs for `setup_split_pane`, grouped to keep its argument count in check.
pub(super) struct TmuxSplitPaneSpec<'a> {
    pub target_pane: &'a str,
    pub cwd: &'a str,
    pub model: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub env_vars: &'a [(&'a str, &'a str)],
    pub background: bool,
}

/// Splits `spec.target_pane` — the tmux pane the invoking process is running
/// in — into a new pane in the same window and starts `claude` there.
pub(super) fn setup_split_pane(spec: TmuxSplitPaneSpec) -> Result<()> {
    let session = tmux::get_session_name_for_pane(spec.target_pane).with_context(|| {
        format!(
            "Failed to resolve tmux session for pane '{}'",
            spec.target_pane
        )
    })?;

    let new_pane_id = tmux::layout::split_pane(tmux::layout::SplitSpec {
        common: tmux::layout::TmuxSessionSpec {
            session: &session,
            cwd: spec.cwd,
            model: spec.model,
            prompt: spec.prompt,
            env_vars: spec.env_vars,
            background: spec.background,
        },
        target_pane: spec.target_pane,
        command: "claude",
    })
    .context("Failed to split tmux pane")?;

    if !spec.background {
        tmux::focus_pane(&new_pane_id).context("Failed to focus new pane")?;
    }

    Ok(())
}
