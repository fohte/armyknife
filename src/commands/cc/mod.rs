mod auto_compact;
mod auto_pause;
mod claude_sessions;
mod error;
mod focus;
mod hook;
mod list;
pub(crate) mod pane_input;
mod resume;
mod resurrect;
mod signal;
pub(crate) mod store;
mod sweep;
mod tui;
pub(crate) mod types;
mod watch;

use clap::Subcommand;

pub use auto_compact::AutoCompactArgs;
pub use focus::FocusArgs;
pub use hook::HookArgs;
pub use list::ListArgs;
pub use resume::ResumeArgs;
pub use resurrect::ResurrectCommands;
pub use sweep::SweepArgs;
pub use watch::WatchArgs;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum CcCommands {
    /// Record Claude Code session events (called from hooks)
    Hook(HookArgs),

    /// List Claude Code sessions
    #[command(visible_alias = "ls")]
    List(ListArgs),

    /// Watch Claude Code sessions in real-time (TUI)
    Watch(WatchArgs),

    /// Focus on a Claude Code session's tmux pane
    Focus(FocusArgs),

    /// Resume a Claude Code session from tmux pane's user option
    #[command(visible_alias = "r")]
    Resume(ResumeArgs),

    /// Save/restore session IDs for tmux-resurrect integration
    #[command(subcommand)]
    Resurrect(ResurrectCommands),

    /// Pause long-stopped sessions by sending SIGTERM (run periodically)
    Sweep(SweepArgs),

    /// Schedule a `/compact` for an idle session while the prompt cache is warm.
    #[command(name = "auto-compact")]
    AutoCompact(AutoCompactArgs),
}

impl CcCommands {
    pub async fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::Hook(args) => hook::run(args)?,
            Self::List(args) => list::run(args)?,
            Self::Watch(args) => watch::run(args)?,
            Self::Focus(args) => focus::run(args)?,
            Self::Resume(args) => resume::run(args)?,
            Self::Resurrect(cmd) => resurrect::run(cmd)?,
            Self::Sweep(args) => sweep::run(args)?,
            Self::AutoCompact(args) => auto_compact::run(args).await?,
        }
        Ok(())
    }
}
