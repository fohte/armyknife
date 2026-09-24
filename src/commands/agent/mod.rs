mod auto_compact;
pub(crate) mod auto_pause;
mod bg;
pub(crate) mod bg_tasks;
pub(crate) mod claude_messaging;
pub(crate) mod claude_registry;
mod claude_sessions;
mod clean_detached;
mod codex;
mod codex_queue;
pub(crate) mod codex_steer;
mod delete_tq_session_detached;
mod error;
mod focus;
mod generate_title_detached;
mod graceful_quit;
mod hook;
mod list;
mod mark_read;
pub(crate) mod new;
pub(crate) mod pane;
pub(crate) mod peer;
mod resume;
mod resurrect;
mod signal;
pub(crate) mod store;
mod sweep;
pub(crate) mod tmux_sync;
mod tui;
pub(crate) mod types;
mod watch;
mod window_status;

use clap::Subcommand;

pub use auto_compact::AutoCompactArgs;
pub use bg::BgCommands;
pub use clean_detached::CleanDetachedArgs;
pub use codex::CodexArgs;
pub use delete_tq_session_detached::DeleteTqSessionDetachedArgs;
pub use focus::FocusArgs;
pub use generate_title_detached::GenerateTitleDetachedArgs;
pub use hook::HookArgs;
pub use hook::permission_notification::DelayedPermissionNotificationArgs;
pub use list::ListArgs;
pub use mark_read::MarkReadArgs;
pub use new::NewArgs;
pub use pane::status::HasPausedArgs;
pub use peer::PeerCommands;
pub use resume::ResumeArgs;
pub use resurrect::ResurrectCommands;
pub use sweep::SweepArgs;
pub use watch::WatchArgs;
pub use window_status::WindowStatusArgs;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum AgentCommands {
    /// Start a Claude Code session in a new worktree
    New(NewArgs),

    /// Start Codex and bind its thread ID to the current tmux pane
    Codex(CodexArgs),

    /// Record Claude Code session events (called from hooks)
    Hook(HookArgs),

    /// Internal: send a delayed permission notification after the
    /// request remains unresolved through the grace period.
    #[command(name = "permission-notification", hide = true)]
    PermissionNotification(DelayedPermissionNotificationArgs),

    /// List Claude Code sessions
    #[command(visible_alias = "ls")]
    List(ListArgs),

    /// Watch Claude Code sessions in real-time (TUI)
    Watch(WatchArgs),

    /// Focus on a Claude Code session's tmux pane
    Focus(FocusArgs),

    /// Mark the pane's Claude Code session as read (wire from tmux pane-focus-in)
    #[command(name = "mark-read")]
    MarkRead(MarkReadArgs),

    /// Resume a Claude Code session from tmux pane's user option
    #[command(visible_alias = "r")]
    Resume(ResumeArgs),

    /// Save/restore session IDs for tmux-resurrect integration
    #[command(subcommand)]
    Resurrect(ResurrectCommands),

    /// Resolve SendMessage target names for related Claude Code sessions
    #[command(subcommand)]
    Peer(PeerCommands),

    /// Pause long-stopped sessions by requesting agent shutdown (run periodically)
    Sweep(SweepArgs),

    /// Run a command in the background and notify this session when it finishes
    #[command(subcommand)]
    Bg(BgCommands),

    /// Schedule a `/compact` for an idle session while the prompt cache is warm.
    #[command(name = "auto-compact")]
    AutoCompact(AutoCompactArgs),

    /// Print status symbols for the Claude Code sessions in a tmux window
    #[command(name = "window-status")]
    WindowStatus(WindowStatusArgs),

    /// Print `1` when the tmux pane carries a Paused Claude Code session,
    /// the empty string otherwise.
    #[command(name = "pane-has-paused")]
    PaneHasPaused(HasPausedArgs),

    /// Internal: non-interactive batch worktree cleanup for `agent watch`.
    #[command(name = "clean-detached", hide = true)]
    CleanDetached(CleanDetachedArgs),

    /// Internal: non-interactive title generation for `agent watch`'s Ctrl+g.
    #[command(name = "generate-title-detached", hide = true)]
    GenerateTitleDetached(GenerateTitleDetachedArgs),

    /// Internal: best-effort tq session deletion, spawned detached when a
    /// session is confirmed Ended (a genuine SessionEnd, or eviction on
    /// tmux pane takeover) so a slow/unreachable tq never blocks Claude
    /// Code's exit.
    #[command(name = "delete-tq-session-detached", hide = true)]
    DeleteTqSessionDetached(DeleteTqSessionDetachedArgs),
}

impl AgentCommands {
    pub async fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::New(args) => new::run(args)?,
            Self::Codex(args) => codex::run(args)?,
            Self::Hook(args) => hook::run(args)?,
            Self::PermissionNotification(args) => hook::permission_notification::run(args)?,
            Self::List(args) => list::run(args)?,
            Self::Watch(args) => watch::run(args)?,
            Self::Focus(args) => focus::run(args)?,
            Self::MarkRead(args) => mark_read::run(args)?,
            Self::Resume(args) => resume::run(args)?,
            Self::Resurrect(cmd) => resurrect::run(cmd)?,
            Self::Peer(cmd) => peer::run(cmd)?,
            Self::Sweep(args) => sweep::run(args)?,
            Self::Bg(cmd) => bg::run(cmd)?,
            Self::AutoCompact(args) => auto_compact::run(args).await?,
            Self::WindowStatus(args) => window_status::run(args)?,
            Self::PaneHasPaused(args) => pane::status::run(args)?,
            Self::CleanDetached(args) => clean_detached::run(args).await?,
            Self::GenerateTitleDetached(args) => generate_title_detached::run(args)?,
            Self::DeleteTqSessionDetached(args) => delete_tq_session_detached::run(args)?,
        }
        Ok(())
    }
}
