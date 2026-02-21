mod claude_sessions;
mod error;
mod focus;
mod generate_label;
mod hook;
mod list;
mod resume;
mod resurrect;
pub(crate) mod store;
mod tui;
mod types;
mod watch;

use clap::Subcommand;

pub use focus::FocusArgs;
pub use generate_label::GenerateLabelArgs;
pub use hook::HookArgs;
pub use list::ListArgs;
pub use resume::ResumeArgs;
pub use resurrect::ResurrectCommands;
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

    /// Resume a Claude Code session from tmux pane's user option
    Resume(ResumeArgs),

    /// Save/restore session IDs for tmux-resurrect integration
    #[command(subcommand)]
    Resurrect(ResurrectCommands),

    /// Generate a label for a session (internal, called by hook in background)
    #[command(hide = true)]
    GenerateLabel(GenerateLabelArgs),
}

impl CcCommands {
    pub fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::Hook(args) => hook::run(args)?,
            Self::List(args) => list::run(args)?,
            Self::Watch(args) => watch::run(args)?,
            Self::Focus(args) => focus::run(args)?,
            Self::Resume(args) => resume::run(args)?,
            Self::Resurrect(cmd) => resurrect::run(cmd)?,
            Self::GenerateLabel(args) => generate_label::run(args)?,
        }
        Ok(())
    }
}
