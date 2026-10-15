mod output;
mod redelivery;
mod reminder;
mod reminder_hook;
mod run;
mod undelivered;
mod worker;

use anyhow::Result;
use clap::Subcommand;

pub use redelivery::RedeliverDetachedArgs;
pub use reminder_hook::ReminderHookArgs;
pub use run::RunArgs;
pub use worker::RunDetachedArgs;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum BgCommands {
    /// Run a command in the background and notify this session when it finishes
    Run(RunArgs),

    /// Internal: wait for a background command and report its result
    #[command(name = "run-detached", hide = true)]
    RunDetached(RunDetachedArgs),

    /// Internal: retry completion notifications saved for a resumed session
    #[command(name = "redeliver-detached", hide = true)]
    RedeliverDetached(RedeliverDetachedArgs),

    /// Internal: wake Claude Code when a background Bash task runs too long
    #[command(name = "reminder-hook", hide = true)]
    ReminderHook(ReminderHookArgs),
}

pub fn run(command: &BgCommands) -> Result<Option<i32>> {
    match command {
        BgCommands::Run(args) => {
            run::run(args)?;
            Ok(None)
        }
        BgCommands::RunDetached(args) => {
            worker::run(args)?;
            Ok(None)
        }
        BgCommands::RedeliverDetached(args) => {
            redelivery::run(args)?;
            Ok(None)
        }
        BgCommands::ReminderHook(args) => reminder_hook::run(args),
    }
}

#[cfg(not(test))]
pub(super) fn spawn_redelivery_if_pending(session_id: &str) {
    redelivery::spawn_if_pending(session_id);
}
