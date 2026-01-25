mod error;
mod focus;
mod hook;
mod list;
mod store;
mod tmux;
mod tty;
mod types;

use clap::Subcommand;

pub use focus::FocusArgs;
pub use hook::HookArgs;
pub use list::ListArgs;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum CcCommands {
    /// Record Claude Code session events (called from hooks)
    Hook(HookArgs),

    /// List Claude Code sessions
    List(ListArgs),

    /// Focus on a Claude Code session's tmux pane
    Focus(FocusArgs),
}

impl CcCommands {
    pub fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::Hook(args) => hook::run(args)?,
            Self::List(args) => list::run(args)?,
            Self::Focus(args) => focus::run(args)?,
        }
        Ok(())
    }
}
