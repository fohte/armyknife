mod error;
mod hook;
mod list;
mod store;
mod tmux;
mod tty;
mod tui;
mod types;
mod watch;

use clap::Subcommand;

pub use hook::HookArgs;
pub use list::ListArgs;
pub use watch::WatchArgs;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum CcCommands {
    /// Record Claude Code session events (called from hooks)
    Hook(HookArgs),

    /// List Claude Code sessions
    List(ListArgs),

    /// Watch Claude Code sessions in real-time (TUI)
    Watch(WatchArgs),
}

impl CcCommands {
    pub fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::Hook(args) => hook::run(args)?,
            Self::List(args) => list::run(args)?,
            Self::Watch(args) => watch::run(args)?,
        }
        Ok(())
    }
}
