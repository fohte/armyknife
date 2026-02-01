mod claude_sessions;
mod error;
mod focus;
mod hook;
mod list;
mod restore;
mod store;
mod tui;
mod types;
mod watch;

use clap::Subcommand;

pub use focus::FocusArgs;
pub use hook::HookArgs;
pub use list::ListArgs;
pub use restore::RestoreArgs;
pub use watch::WatchArgs;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum CcCommands {
    /// Record Claude Code session events (called from hooks)
    Hook(HookArgs),

    /// List Claude Code sessions
    List(ListArgs),

    /// Watch Claude Code sessions in real-time (TUI)
    Watch(WatchArgs),

    /// Focus on a Claude Code session's tmux pane
    Focus(FocusArgs),

    /// Restore a Claude Code session from tmux pane title
    Restore(RestoreArgs),
}

impl CcCommands {
    pub fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::Hook(args) => hook::run(args)?,
            Self::List(args) => list::run(args)?,
            Self::Watch(args) => watch::run(args)?,
            Self::Focus(args) => focus::run(args)?,
            Self::Restore(args) => restore::run(args)?,
        }
        Ok(())
    }
}
