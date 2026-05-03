//! `a cc auto-compact` subcommand.
//!
//! Stop hook spawns one detached `schedule` process per Stop event. Each
//! waits `idle_timeout` (anchored on the Stop time, set slightly below the
//! prompt cache TTL) and, if the session is still idle, SIGTERMs the live
//! `claude` and runs `claude -r <id> -p "/compact"` so the compaction happens
//! while the cache is still warm.
//!
//! Pure decision logic lives in [`decision`]; everything time- or process-
//! related lives in [`schedule`] so the policy is unit-testable without tmux,
//! processes, or the network.

pub(crate) mod decision;
mod schedule;

use anyhow::Result;
use clap::{Args, Subcommand};

pub use schedule::spawn_in_background;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct AutoCompactArgs {
    #[command(subcommand)]
    pub command: AutoCompactCommands,
}

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum AutoCompactCommands {
    /// Wait for the configured idle timeout and run `/compact` if the session
    /// is still idle. Spawned by the Stop hook; not intended to be invoked
    /// directly by users.
    Schedule(schedule::ScheduleArgs),
}

pub async fn run(args: &AutoCompactArgs) -> Result<()> {
    match &args.command {
        AutoCompactCommands::Schedule(args) => schedule::run(args).await,
    }
}
