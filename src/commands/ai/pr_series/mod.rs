pub(super) mod model;
mod plan;
mod render;
pub(super) mod storage;
pub(super) mod sync;

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum PrSeriesCommands {
    /// Register or replace a task's validated PR series plan
    Set(plan::SetArgs),

    /// Synchronize the plan section in linked pull request bodies
    Sync(sync::SyncArgs),
}

impl PrSeriesCommands {
    pub async fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::Set(args) => plan::run(args).await,
            Self::Sync(args) => sync::run(args).await,
        }
    }
}
